//! Parent-owned expiry supervision, without model, target or budget authority.
use super::{
    attempts,
    lease::{self, CoordinatorLease},
    scheduler::ScheduledChild,
    supervision_ticket::{SupervisionState, SupervisionTicket, ThreadLife},
};
use crate::agent_runtime::contract::AgentRole;
use super::parent_invocation_owner::ParentInvocationOwner;
use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use std::{
    path::Path,
    sync::{mpsc, Arc},
    thread::JoinHandle,
    time::Duration,
};

pub(super) mod expiry_writer;

pub(crate) struct WorkerSupervisor {
    stop: mpsc::SyncSender<()>,
    state: Arc<SupervisionState>,
    worker: Option<JoinHandle<()>>,
    _owner: ParentInvocationOwner,
}

impl WorkerSupervisor {
    pub(crate) fn start(path: &Path, actor: &CoordinatorLease) -> Result<Self, String> {
        let owner=ParentInvocationOwner::claim(path,&actor.scan_id,actor.attempt_number,&actor.root_run_id)?;
        Self::start_owned(path,actor,owner)
    }

    pub(crate) fn start_owned(path:&Path,actor:&CoordinatorLease,owner:ParentInvocationOwner)->Result<Self,String> {
        Self::start_owned_in(path,actor,owner,None)
    }
    pub(crate) fn start_owned_signed(path:&Path,actor:&CoordinatorLease,owner:ParentInvocationOwner,issuer:crate::agent_runtime::web_mode::withdrawal_issuer::WithdrawalIssuer)->Result<Self,String> {
        Self::start_owned_in(path,actor,owner,Some(issuer))
    }
    fn start_owned_in(path:&Path,actor:&CoordinatorLease,owner:ParentInvocationOwner,issuer:Option<crate::agent_runtime::web_mode::withdrawal_issuer::WithdrawalIssuer>)->Result<Self,String> {
        owner.validate(path,actor)?;
        // Open only an existing database. A mistyped path must never create a
        // second business database or run schema initialization on a worker.
        let mut db = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )
        .map_err(|e| format!("worker_supervisor_open:{e}"))?;
        db.busy_timeout(Duration::from_millis(250))
            .map_err(|e| e.to_string())?;
        db.pragma_update(None, "foreign_keys", true)
            .map_err(|e| e.to_string())?;
        crate::collaboration_events::install_commit_notifications(&db);
        validate_parent(&db, actor)?;
        require_original_issuer(&db, actor, issuer.as_ref())?;
        // Scope by immutable Root, not epoch/fence: a replacement Coordinator
        // must not overlap the original service that has not finished joining.
        // Claim before the first renewal/withdrawal; a duplicate has no cleanup.
        // Already held by this exact original scope, never claimed twice.
        let state = SupervisionState::new(path, actor)?;
        sweep(&mut db, actor, &state.ticket(), issuer.as_ref())?;
        let (stop, receiver) = mpsc::sync_channel(1);
        let thread_state = state.clone();
        let actor = actor.clone();
        let worker = std::thread::Builder::new()
            .name("native-worker-supervisor".into())
            .spawn(move || {
                let life = ThreadLife(thread_state);
                loop {
                    match receiver.recv_timeout(Duration::from_millis(100)) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    if let Err(error) = sweep(&mut db, &actor, &life.0.ticket(), issuer.as_ref()) {
                        life.0.fail(error);
                        break;
                    }
                }
            })
            .map_err(|e| format!("worker_supervisor_thread:{e}"))?;
        Ok(Self {
            stop,
            state,
            worker: Some(worker),
            _owner: owner,
        })
    }

    pub(crate) fn check(&self) -> Result<(), String> {
        self.ticket().check()?;
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            return Err("worker_supervisor_stopped".into());
        }
        Ok(())
    }

    pub(crate) fn ticket(&self) -> SupervisionTicket {
        self.state.ticket()
    }
}

impl Drop for WorkerSupervisor {
    fn drop(&mut self) {
        self.state.stop();
        let _ = self.stop.try_send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

// Pure snapshot admission before every heartbeat, including unsigned callers.
fn require_original_issuer(
    db: &Connection,
    actor: &CoordinatorLease,
    issuer: Option<&crate::agent_runtime::web_mode::withdrawal_issuer::WithdrawalIssuer>,
) -> Result<(), String> {
    if db.is_autocommit() {
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        return require_original_issuer(&tx, actor, issuer);
    }
    let declaration = crate::agent_runtime::web_mode::root::read(db, &actor.root_run_id)?;
    let required = declaration.as_ref().is_some_and(|d| d.lease_expiry_supervision().is_some());
    if required && issuer.is_none() {
        return Err("worker_supervisor_original_signed_issuer_required".into());
    }
    if let Some(issuer) = issuer {
        issuer.require_actor(db, actor)?;
    }
    Ok(())
}

fn sweep(
    db: &mut Connection,
    actor: &CoordinatorLease,
    parent: &SupervisionTicket,
    issuer: Option<&crate::agent_runtime::web_mode::withdrawal_issuer::WithdrawalIssuer>,
) -> Result<(), String> {
    validate_parent(db, actor)?;
    require_original_issuer(db, actor, issuer)?;
    super::coordinator_heartbeat::renew_if_due(db, actor, parent)?;
    let missing: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments a
        WHERE a.coordinator_run_id=?1 AND a.state IN ('leased','running','waiting_review','paused')
          AND a.scheduling_model='legacy_role_pipeline'
          AND a.budget_settled_at='' AND (SELECT count(*) FROM agent_assignment_attempts x
            WHERE x.assignment_id=a.id AND x.child_run_id=a.child_run_id AND x.root_run_id=?1)<>1)",
            [&actor.root_run_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("worker_supervisor_workers:{e}"))?;
    if missing {
        return Err("worker_supervisor_worker_missing_or_conflicting".into());
    }
    let due: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments a
        JOIN agent_assignment_attempts x ON x.assignment_id=a.id AND x.child_run_id=a.child_run_id
        WHERE a.coordinator_run_id=?1 AND x.state IN ('leased','running','paused')
          AND a.scheduling_model='legacy_role_pipeline'
          AND COALESCE(datetime(x.expires_at)<=datetime('now','localtime'),1))",
            [&actor.root_run_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("worker_supervisor_deadlines:{e}"))?;
    // Phase 1.1（REVIEW §8.1 第 2 条）：同质载体的到期回收挂在同一条心跳上。
    // 它不能进 `expiry_writer::write`——那个私有写者只允许 legacy 提款的表/列，
    // 且 `attempts/expiry.rs` 要求 agent_lane_leases 与旧状态机，同质行两者都没有。
    // 回收失败不推翻监护线程：这是一张清洁工，主管轮开头还会以同一函数再收一次。
    reclaim_homogeneous_expired(db, actor);
    if !due {
        return Ok(());
    }
    expiry_writer::write(db, |db, authority| withdraw_due(db, actor, authority, issuer))
}

/// 同质载体到期回收：先用一次只读判定，确认真的有过期载体才开写事务。
/// 心跳每 100ms 一跳，legacy 扫描绝不能因此多抢一次写锁（会把手臂长的管道测打成
/// `database is locked`）；这与 legacy 侧 `due` 的门控纪律保持一致。
fn reclaim_homogeneous_expired(db: &mut Connection, actor: &CoordinatorLease) {
    use crate::agent_runtime::multi_agent::homogeneous::pool;
    let due: bool = match db.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignments a
            JOIN agent_assignment_attempts x ON x.assignment_id=a.id AND x.child_run_id=a.child_run_id
            WHERE a.coordinator_run_id=?1 AND a.scheduling_model='homogeneous_workers'
              AND x.state IN ('leased','running','paused')
              AND datetime(x.expires_at)<=datetime('now','localtime'))",
        [&actor.root_run_id],
        |row| row.get(0),
    ) {
        Ok(due) => due,
        Err(_) => return,
    };
    if !due {
        return;
    }
    // pool owns the atomic reclaim transaction. Opening an outer transaction
    // here made its BEGIN fail and silently left expired carriers occupied.
    let _ = pool::reclaim_expired_for_root(db, &actor.root_run_id);
}

fn withdraw_due(
    db: &mut Connection, actor: &CoordinatorLease,
    authority: &expiry_writer::ExpiryWriteAuthority,
    issuer: Option<&crate::agent_runtime::web_mode::withdrawal_issuer::WithdrawalIssuer>,
) -> Result<(), String> {
    // A healthy foreground transaction may own the SQLite writer. Workers'
    // existing deadline gates already prohibit expired execution while the
    // supervisor waits to withdraw. Retry contention, never partial writes.
    let tx = match db.transaction_with_behavior(TransactionBehavior::Immediate) {
        Ok(tx) => tx,
        Err(rusqlite::Error::SqliteFailure(error, _))
            if matches!(
                error.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            ) =>
        {
            return Ok(())
        }
        Err(error) => return Err(format!("worker_supervisor_lock:{error}")),
    };
    authority.require_snapshot(&tx)?;
    validate_parent(&tx, actor)?;
    crate::collaboration_events::closure_schema::verify(&tx)?;
    let children = {
        let mut stmt = tx.prepare("SELECT a.id,a.child_run_id,a.role FROM agent_assignments a
            JOIN agent_assignment_attempts x ON x.assignment_id=a.id AND x.child_run_id=a.child_run_id
            WHERE a.coordinator_run_id=?1 AND x.state IN ('leased','running','paused')
              AND a.scheduling_model='legacy_role_pipeline'
              AND COALESCE(datetime(x.expires_at)<=datetime('now','localtime'),1)
            ORDER BY a.id").map_err(|e| format!("worker_supervisor_read:{e}"))?;
        let rows = stmt
            .query_map([&actor.root_run_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut children = Vec::new();
        for row in rows {
            let (assignment_id, run_id, role) = row.map_err(|e| e.to_string())?;
            let parsed = AgentRole::try_parse(&role)
                .filter(|parsed| parsed.as_str() == role && *parsed != AgentRole::Coordinator)
                .ok_or("worker_supervisor_role_invalid")?;
            children.push(ScheduledChild {
                assignment_id,
                run_id,
                role: parsed,
            });
        }
        children
    };
    for child in children {
        let withdrawal = attempts::withdraw_new_for_supervision(&tx, actor, &child, authority)?;
        // Selected under this same writer lock: only a new original withdrawal
        // can be certified. Old expired history cannot become an issued proof.
        let withdrawal = withdrawal.ok_or("worker_supervisor_expiry_unconfirmed")?;
        withdrawal.persist_receipt(&tx, authority, issuer)?;
    }
    // Recheck after trigger-backed withdrawals, before committing any worker.
    validate_parent(&tx, actor)?;
    crate::collaboration_events::closure_schema::verify(&tx)?;
    tx.commit()
        .map_err(|e| format!("worker_supervisor_commit:{e}"))
}

fn validate_parent(db: &Connection, actor: &CoordinatorLease) -> Result<(), String> {
    lease::validate_coordinator_lease(db, actor)?;
    lease::require_executable_coordinator(db, actor)?;
    let native: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_runs
        WHERE id=?1 AND root_run_id=id AND parent_run_id IS NULL AND assignment_id=''
          AND role='coordinator' AND backend='native' AND orchestration_policy='multi'
          AND scan_id=?2 AND attempt_number=?3 AND target_url=?4)",
            params![
                actor.root_run_id,
                actor.scan_id,
                actor.attempt_number,
                actor.target_key
            ],
            |row| row.get(0),
        )
        .map_err(|e| format!("worker_supervisor_parent:{e}"))?;
    if !native {
        return Err("worker_supervisor_parent_binding_conflict".into());
    }
    super::budget::clock::supervision_remaining(db, &actor.root_run_id)?;
    Ok(())
}
