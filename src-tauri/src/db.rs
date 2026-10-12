use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde_json::json;
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const DEFAULT_BUSINESS_FRONTEND_SKILL: &str =
    include_str!("../resources/skills/business_frontend_deep_analysis.md");

pub fn open(path: &Path) -> Result<Connection, String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|error| error.to_string())?;
    connection
        .busy_timeout(Duration::from_secs(10))
        .map_err(|error| error.to_string())?;
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(|error| error.to_string())?;
    // journal_mode is persistent and is enabled once during initialize().
    // Re-applying it for every UI command asks SQLite for a write lock. While a
    // collector is writing this made otherwise read-only page changes wait for
    // the full busy timeout and looked like the application had frozen.
    connection
        .pragma_update(None, "synchronous", "NORMAL")
        .map_err(|error| error.to_string())?;
    crate::collaboration_events::install_commit_notifications(&connection);
    Ok(connection)
}

fn environment_preparation_lock(path: &Path) -> Result<fs::File, String> {
    let lock_path = path.with_extension("preparation.lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|error| format!("无法打开环境准备锁 {}：{error}", lock_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("无法保护环境准备锁：{error}"))?;
    }
    Ok(file)
}

/// Bounded wait for references to a just-released preparation lock held only
/// by children that another thread is spawning right now. std opens the lock
/// file with O_CLOEXEC, but a spawned child shares every open file description
/// from fork/spawn until exec closes it, so for a few milliseconds a released
/// flock still reads as held (measured on macOS with 4 concurrent spawners:
/// ~3% of immediate re-locks failed). A real preparation holds the lock for the
/// whole install, far longer than this grace, so it is still refused.
const PREPARATION_LOCK_SPAWN_SETTLE: Duration = Duration::from_millis(200);

fn try_lock_settled(file: &fs::File) -> Result<(), fs::TryLockError> {
    let deadline = Instant::now() + PREPARATION_LOCK_SPAWN_SETTLE;
    loop {
        match file.try_lock() {
            Err(fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(2));
            }
            result => return result,
        }
    }
}

/// The OS lock is released on process death. The SQLite row is the durable
/// fence seen by every scan activation, including other application processes.
pub struct EnvironmentPreparationLease {
    db_path: PathBuf,
    owner: String,
    _lock: fs::File,
    #[cfg(unix)]
    original_database: fs::File,
}

impl EnvironmentPreparationLease {
    /// Authority comes from this held OS guard and its exact durable owner,
    /// never from the mere presence of a preparation row.
    pub(crate) fn require_tool_supply_on(
        &self,
        tx: &rusqlite::Transaction<'_>,
    ) -> Result<(), String> {
        let rejected = || "tool_supply_preparation_owner_mismatch".to_string();
        let path = tx.path().filter(|path| !path.is_empty()).ok_or_else(rejected)?;
        if fs::canonicalize(path).map_err(|_| rejected())?
            != fs::canonicalize(&self.db_path).map_err(|_| rejected())?
        {
            return Err(rejected());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let original = self.original_database.metadata().map_err(|_| rejected())?;
            let database = fs::metadata(path).map_err(|_| rejected())?;
            if original.dev() != database.dev() || original.ino() != database.ino() {
                return Err(rejected());
            }
            // Ask SQLite about its opened entity too: checking the current
            // pathname alone misses a connection still holding a renamed DB.
            let mut moved: std::os::raw::c_int = 0;
            let checked = unsafe {
                rusqlite::ffi::sqlite3_file_control(
                    tx.handle(), c"main".as_ptr(),
                    rusqlite::ffi::SQLITE_FCNTL_HAS_MOVED,
                    (&mut moved as *mut std::os::raw::c_int).cast(),
                )
            };
            if checked != rusqlite::ffi::SQLITE_OK || moved != 0 {
                return Err(rejected());
            }
            let held = self._lock.metadata().map_err(|_| rejected())?;
            let current = fs::symlink_metadata(self.db_path.with_extension("preparation.lock"))
                .map_err(|_| rejected())?;
            if !current.is_file() || current.file_type().is_symlink()
                || held.dev() != current.dev() || held.ino() != current.ino()
            {
                return Err(rejected());
            }
        }
        let shadowed: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE name IN ('environment_preparation_lease','sentinel_scans'))",
            [], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        if shadowed { return Err(rejected()); }
        let owns: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM main.environment_preparation_lease WHERE singleton=1 AND owner=?1) AND NOT EXISTS(SELECT 1 FROM main.sentinel_scans WHERE status IN ('queued','scanning','pausing'))",
            [&self.owner], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        if !owns { return Err(rejected()); }
        Ok(())
    }

    /// An inert tool candidate is not an approved or runnable capability. Log
    /// its exact digest and identity key while this process still owns the
    /// preparation fence. Different signed releases may share package bytes.
    pub(crate) fn record_tool_candidate(
        &self,
        tool_id: &str,
        digest: &str,
        identity_key: &str,
        already_present: bool,
    ) -> Result<(), String> {
        let event = format!(
            "tool_candidate_{}:{tool_id}:{digest}:{identity_key}",
            if already_present {
                "revalidated"
            } else {
                "staged"
            }
        );
        let connection = open(&self.db_path)?;
        let inserted = connection
            .execute(
                "INSERT INTO environment_preparation_events(owner,event) SELECT owner,?2 FROM environment_preparation_lease WHERE singleton=1 AND owner=?1",
                params![self.owner, event],
            )
            .map_err(|error| error.to_string())?;
        if inserted != 1 {
            return Err("环境准备租约已变化，不能记录能力包候选审计".into());
        }
        Ok(())
    }

    /// Only the caller that observed a complete preparation may release the
    /// durable fence. Dropping the guard (including unwinding) releases the OS
    /// lock but deliberately leaves the row for manual recovery.
    pub fn complete(self) -> Result<(), String> {
        let mut connection = open(&self.db_path)?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        self.require_tool_supply_on(&tx)?;
        let removed = tx
            .execute(
                "DELETE FROM environment_preparation_lease WHERE singleton=1 AND owner=?1",
                [&self.owner],
            )
            .map_err(|error| error.to_string())?;
        if removed != 1 {
            return Err("环境准备租约已变化，不能标记为完成".into());
        }
        tx.execute(
            "INSERT INTO environment_preparation_events(owner,event) VALUES(?1,'completed')",
            [&self.owner],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())
    }
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentPreparationStatus {
    pub state: &'static str,
    pub owner: Option<String>,
    pub created_at: Option<String>,
}

/// A free OS lock is not evidence that an installer child has exited. Keep
/// the durable fence visible until an operator explicitly recovers it.
pub fn environment_preparation_status(path: &Path) -> Result<EnvironmentPreparationStatus, String> {
    let lock = environment_preparation_lock(path)?;
    let lock_available = try_lock_settled(&lock).is_ok();
    let connection = open(path)?;
    let lease: Option<(String, String)> = connection
        .query_row(
            "SELECT owner,created_at FROM environment_preparation_lease WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let (owner, created_at) = match lease {
        Some((owner, created_at)) => (Some(owner), Some(created_at)),
        None => (None, None),
    };
    Ok(EnvironmentPreparationStatus {
        state: if !lock_available {
            "installing"
        } else if owner.is_some() {
            "requires_manual_recovery"
        } else {
            "idle"
        },
        owner,
        created_at,
    })
}

/// The acknowledgement is a human attestation, not a process-verification
/// mechanism. Recovery is refused while the OS lock is held or scans are live.
pub fn recover_environment_preparation(
    path: &Path,
    expected_owner: &str,
    acknowledgement: &str,
) -> Result<(), String> {
    if expected_owner.is_empty() || acknowledgement != "已确认安装子进程停止" {
        return Err("需核对租约并输入完整确认语，确认所有安装子进程已经停止".into());
    }
    let lock = environment_preparation_lock(path)?;
    try_lock_settled(&lock).map_err(|_| "环境准备仍在运行，不能解除锁定".to_string())?;
    let mut connection = open(path)?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let active: i64 = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE status IN ('queued','scanning','pausing'))",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if active != 0 {
        return Err("仍有活跃或排队的扫描，不能恢复环境准备租约".into());
    }
    let removed = tx
        .execute(
            "DELETE FROM environment_preparation_lease WHERE singleton=1 AND owner=?1",
            [expected_owner],
        )
        .map_err(|error| error.to_string())?;
    if removed != 1 {
        return Err("环境准备租约已变化；请重新获取状态后核对".into());
    }
    tx.execute(
        "INSERT INTO environment_preparation_events(owner,event) VALUES(?1,'manual_recovered_installer_verified')",
        [expected_owner],
    )
    .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())
}

/// Claim preparation and check the queue in one SQLite writer transaction.
/// No task can enter queued/scanning until this guard is completed. A process
/// crash leaves the durable row fenced: child installers may outlive us, so an
/// absent OS lock alone is insufficient proof that it is safe to recover.
pub fn begin_environment_preparation(path: &Path) -> Result<EnvironmentPreparationLease, String> {
    let lock = environment_preparation_lock(path)?;
    try_lock_settled(&lock)
        .map_err(|error| format!("已有环境准备正在进行，拒绝同时安装：{error}"))?;
    let mut connection = open(path)?;
    #[cfg(unix)]
    let original_database = {
        use std::os::unix::fs::MetadataExt;
        let file = fs::File::open(path).map_err(|error| error.to_string())?;
        let held = file.metadata().map_err(|error| error.to_string())?;
        let current = fs::metadata(path).map_err(|error| error.to_string())?;
        if held.dev() != current.dev() || held.ino() != current.ino() {
            return Err("environment_preparation_database_changed".into());
        }
        file
    };
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let active: i64 = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE status IN ('queued','scanning','pausing'))",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if active != 0 {
        return Err(
            "存在等待或正在执行的扫描任务；请先完成或暂停并移出队列，再由管理员安装环境依赖".into(),
        );
    }
    if tx
        .query_row(
            "SELECT owner FROM environment_preparation_lease WHERE singleton=1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Err(
            "上次环境准备未正常结束；可能仍有安装子进程，拒绝新任务与再次安装，需管理员核对后恢复"
                .into(),
        );
    }
    let owner = uuid::Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO environment_preparation_lease(singleton,owner) VALUES(1,?1)",
        [&owner],
    )
    .map_err(|error| error.to_string())?;
    tx.execute(
        "INSERT INTO environment_preparation_events(owner,event) VALUES(?1,'started')",
        [&owner],
    )
    .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(EnvironmentPreparationLease {
        db_path: path.to_path_buf(),
        owner,
        _lock: lock,
        #[cfg(unix)]
        original_database,
    })
}

#[path = "db_settings.rs"]
mod settings_migration;
pub use settings_migration::normalize_settings;
#[path = "agent_runtime/multi_agent/attempts/schema.rs"]
mod assignment_attempt_schema;
#[path = "agent_runtime/multi_agent/budget/schema.rs"]
mod budget_schema;

pub fn settings_for_update(
    stored: &serde_json::Value,
    incoming: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    settings_migration::settings_for_update(stored, incoming)
}

pub fn initialize(app_data_dir: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(app_data_dir).map_err(|error| error.to_string())?;
    let path = app_data_dir.join("oviraptor.sqlite3");
    let mut connection = open(&path)?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    connection
        .execute_batch(SCHEMA)
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(budget_schema::SCHEMA)
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(include_str!("agent_runtime/web_mode/schema.sql"))
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(include_str!("commands/native_protection_recovery_schema.sql"))
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(include_str!("commands/native_protection_recovery_priced_schema.sql"))
        .map_err(|error| error.to_string())?;
    connection.execute_batch(include_str!("agent_runtime/web_mode/root/birth_seal.sql"))
        .map_err(|e| e.to_string())?;
    connection
        .execute_batch(include_str!("agent_runtime/multi_agent/budget/root/model/tick/schema.sql"))
        .map_err(|error|error.to_string())?;
    connection
        .execute_batch(include_str!("agent_runtime/multi_agent/budget/root/model/tick/confirmation/schema.sql"))
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(assignment_attempt_schema::SCHEMA)
        .map_err(|error| error.to_string())?;
    connection.execute_batch(include_str!("agent_runtime/multi_agent/attempts/withdrawal_receipt/schema.sql"))
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(include_str!("native_pipeline/process_log/schema.sql"))
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(include_str!(
            "agent_runtime/multi_agent/directive/human_review/schema.sql"
        ))
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(include_str!(
            "agent_runtime/multi_agent/directive/proposals/owned/schema.sql"
        ))
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(include_str!("agent_runtime/model/diagnostics/schema.sql"))
        .map_err(|error| error.to_string())?;
    connection.execute_batch(include_str!("agent_runtime/deleted_scan_audit/schema.sql")).map_err(|e|e.to_string())?;
    connection.execute_batch(include_str!("agent_runtime/multi_agent/budget/clock/finalization/exit_receipt/schema.sql")).map_err(|e|e.to_string())?;
    connection.execute_batch(crate::agent_runtime::multi_agent::specialist::mapper_loop::SCHEMA)
        .map_err(|e|format!("无法初始化 Mapper 多轮原费用账据：{e}"))?;
    connection.execute_batch(crate::agent_runtime::multi_agent::specialist::mapper_loop::final_seal::FINAL_SCHEMA)
        .map_err(|e|format!("无法初始化 Mapper 多轮最终原收据：{e}"))?;
    connection.execute_batch(crate::agent_runtime::multi_agent::specialist::mapper_loop::expired_seal::EXPIRED_SCHEMA)
        .map_err(|e|format!("无法初始化 Mapper 过期原输出收据：{e}"))?;
    connection.execute_batch(include_str!("tool_supply/approval/schema.sql"))
        .map_err(|e|format!("无法初始化工具管理员批准库存：{e}"))?;
    connection.execute_batch(include_str!("tool_supply/frozen_plan/schema.sql"))
        .map_err(|e|format!("无法初始化任务工具冻结计划：{e}"))?;
    connection.execute_batch(include_str!("tool_supply/selection/schema.sql"))
        .map_err(|e|format!("无法初始化新草稿工具准备选择：{e}"))?;
    migrate_import_membership_source_paths(&connection)?;
    suspend_collaboration_event_triggers(&connection)?;
    migrate_legacy_agent_and_retry_columns(&mut connection)?;
    verify_locked_platform_schema(&mut connection)?;
    // Live knowledge uses neutral tables. Startup must not import or mutate retired
    // backend rows: old data is not part of the Native runtime contract.
    connection.execute_batch(include_str!("agent_runtime/multi_agent/directive/ordered_execution/schema.sql"))
        .map_err(|e|format!("无法初始化有序评估回执：{e}"))?;
    connection
        .execute_batch(NEUTRAL_KNOWLEDGE_SCHEMA)
        .map_err(|error| error.to_string())?;
    migrate_agent_run_orchestration_columns(&mut connection)?;
    worker_namespaces::migrate(&mut connection)?;
    migrate_evidence_revision_schema(&connection)?;
    migrate_agent_review_verdict_vocabulary(&mut connection)?;
    migrate_fuse_entry_status(&mut connection)?;
    migrate_builtin_prompts_and_recon_routes(&mut connection)?;
    seed_builtin_agent_skill(&mut connection)?;
    migrate_targets_and_opportunities(&mut connection)?;
    reconcile_investigation_source_keys(&mut connection)?;
    repair_asset_duplicates_and_probe_labels(&mut connection)?;
    migrate_budget_defaults(&mut connection)?;
    crate::collaboration_events::ensure_schema(&connection)?;
    connection.execute_batch(include_str!("agent_runtime/multi_agent/budget/root/model/tick/timeline_event_schema.sql"))
        .map_err(|error| error.to_string())?;
    Ok(path)
}

/// A content-addressed bundle may be imported from several directories. The
/// bundle's latest source_path cannot identify the source of older memberships.
/// Existing rows can only be backfilled when their bundle has a single scope.
/// For an ambiguous old hash shared by multiple scopes, leave the path empty
/// rather than misattribute one scan's historical claims to another scan.
/// New rows capture the exact path when the projection is committed.
fn migrate_import_membership_source_paths(connection: &Connection) -> Result<(), String> {
    ensure_column(
        connection,
        "import_projection_memberships",
        "source_path",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    connection.execute(
        "UPDATE import_projection_memberships SET source_path=(\
            SELECT b.source_path FROM import_bundles b WHERE b.id=import_projection_memberships.bundle_row_id\
         ) WHERE source_path='' AND bundle_row_id IN (
            SELECT bundle_row_id FROM import_projection_memberships
            GROUP BY bundle_row_id HAVING COUNT(DISTINCT scope_key)=1
         )",
        [],
    ).map_err(|error| format!("无法迁移历史投影来源目录：{error}"))?;
    Ok(())
}

// §12: the migration helpers, the schema text and the tests live in their own
// files. These are textual includes of the same module, so nothing had to be made
// public or duplicated.
include!("db_migrate.rs");
include!("db_schema.rs");
include!("db_initialize.rs");
#[path = "db/worker_namespaces.rs"]
mod worker_namespaces;
#[cfg(test)]
pub(crate) fn migrate_worker_namespaces_for_test(
    connection: &mut Connection,
) -> Result<(), String> {
    worker_namespaces::migrate(connection)
}
include!("db_neutral.rs");

#[cfg(test)]
mod tests {
    // §12: the migration tests live in their own file; this is a textual include, so
    // they still reach the private helpers of `db` through `super::*`.
    include!("db_tests.rs");
    include!("db_settings_startup_tests.rs");
    include!("db_neutral_tests.rs");
    include!("db_startup_schema_tests.rs");
    include!("db_sqlite_concurrency_tests.rs");
    include!("db_preparation_lock_spawn_tests.rs");
    include!("db_tool_supply_lease_tests.rs");
}
