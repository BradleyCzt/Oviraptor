//! Typed original SDK ownership, read from an existing claimed dispatch only.
use crate::agent_runtime::{
    multi_agent::{lease::CoordinatorLease, scheduler::ScheduledChild},
    store::stable_hash,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Domain {
    Root,
    Specialist,
    SourceRound,
    MapperLoop,
}
impl Domain {
    pub(super) fn storage(self) -> Self {
        if self == Self::MapperLoop {
            Self::Specialist
        } else {
            self
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Specialist => "specialist",
            Self::SourceRound => "source_round",
            Self::MapperLoop => "mapper_loop",
        }
    }
}
impl rusqlite::types::ToSql for Domain {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(self.as_str().into())
    }
}
impl rusqlite::types::FromSql for Domain {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        match value.as_str()? {
            "root" => Ok(Self::Root),
            "specialist" => Ok(Self::Specialist),
            "source_round" => Ok(Self::SourceRound),
            "mapper_loop" => Ok(Self::MapperLoop),
            _ => Err(rusqlite::types::FromSqlError::InvalidType),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Owner {
    pub owner_id: String,
    pub domain: Domain,
    pub dispatch_key: String,
    pub scan_id: String,
    pub attempt: i64,
    pub root_run_id: String,
    pub run_id: String,
    pub assignment_id: Option<String>,
    pub lease_attempt_id: String,
    pub worker_id: Option<String>,
    pub round: i64,
    pub request_hash: String,
}
impl Owner {
    pub(super) fn root(db: &Connection, run: &str, round: i64) -> Result<Self, String> {
        let row: (String,i64,String,String,String)=db.query_row("SELECT r.scan_id,r.attempt_number,j.call_id,j.lease_attempt_id,j.request_hash
    FROM agent_root_model_journal j JOIN agent_runs r ON r.id=j.root_run_id
    JOIN agent_root_budget_attempts x ON x.id=j.lease_attempt_id AND x.root_run_id=r.id
    WHERE r.id=?1 AND j.round=?2 AND j.phase='dispatch' AND r.backend='native'
      AND r.orchestration_policy='single' AND r.role='coordinator' AND r.assignment_id=''
      AND r.parent_run_id IS NULL AND json_extract(x.contract_json,'$.root.scan')=r.scan_id
      AND json_extract(x.contract_json,'$.root.attempt')=r.attempt_number AND NOT EXISTS(SELECT 1 FROM agent_root_model_journal t WHERE t.call_id=j.call_id AND t.phase<>'dispatch')",
    params![run,round],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))
    .map_err(|_|"native_sdk_log_original_dispatch_missing")?;
        let mut owner = Self {
            owner_id: String::new(),
            domain: Domain::Root,
            dispatch_key: row.2,
            scan_id: row.0,
            attempt: row.1,
            root_run_id: run.into(),
            run_id: run.into(),
            assignment_id: None,
            lease_attempt_id: row.3,
            worker_id: None,
            round,
            request_hash: row.4,
        };
        owner.seal()?;
        Ok(owner)
    }
    pub(super) fn child(
        db: &Connection,
        lease: &CoordinatorLease,
        child: &ScheduledChild,
        round: i64,
        tool: bool,
    ) -> Result<Self, String> {
        let table = if tool {
            "agent_source_model_rounds"
        } else {
            "agent_specialist_calls"
        };
        let round_sql = if tool { "c.round_number=?6" } else { "1=?6" };
        let sql=format!("SELECT c.request_hash,x.id,x.worker_id FROM {table} c JOIN agent_runs r ON r.id=c.child_run_id
    JOIN agent_assignment_attempts x ON x.child_run_id=r.id AND x.assignment_id=c.assignment_id AND x.root_run_id=c.root_run_id
    WHERE c.assignment_id=?1 AND c.child_run_id=?2 AND c.root_run_id=?3 AND c.role=?4
      AND c.lease_epoch=?5 AND {round_sql} AND c.fencing_token=?7 AND x.coordinator_epoch=c.lease_epoch
      AND x.coordinator_fencing_token=c.fencing_token AND r.root_run_id=c.root_run_id AND r.assignment_id=c.assignment_id
      AND r.scan_id=?8 AND r.attempt_number=?9 AND r.role=c.role AND c.state='executing'
      AND c.response_json='{{}}' AND c.usage_json='{{}}' AND c.response_hash='' AND c.event_sequence=0 AND c.failure_code=''");
        let row: (String, String, String) = db
            .query_row(
                &sql,
                params![
                    child.assignment_id,
                    child.run_id,
                    lease.root_run_id,
                    child.role.as_str(),
                    lease.lease_epoch,
                    round,
                    lease.fencing_token,
                    lease.scan_id,
                    lease.attempt_number
                ],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|_| "native_sdk_log_original_dispatch_missing")?;
        let domain = if tool {
            Domain::SourceRound
        } else {
            Domain::Specialist
        };
        // The key identifies the original physical model row; it is no new claim.
        let key = stable_hash(
            &json!([domain, child.assignment_id, child.run_id, round, row.0]).to_string(),
        );
        let mut owner = Self {
            owner_id: String::new(),
            domain,
            dispatch_key: key,
            scan_id: lease.scan_id.clone(),
            attempt: lease.attempt_number,
            root_run_id: lease.root_run_id.clone(),
            run_id: child.run_id.clone(),
            assignment_id: Some(child.assignment_id.clone()),
            lease_attempt_id: row.1,
            worker_id: Some(row.2),
            round,
            request_hash: row.0,
        };
        owner.seal()?;
        Ok(owner)
    }
    pub(super) fn seal(&mut self) -> Result<(), String> {
        let ids = [
            self.scan_id.as_str(),
            self.root_run_id.as_str(),
            self.run_id.as_str(),
            self.lease_attempt_id.as_str(),
        ];
        if self.attempt < 1
            || self.round < 1
            || ids.iter().any(|s| {
                s.is_empty()
                    || s.len() > 128
                    || !s
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_:".contains(&b))
            })
            || [&self.dispatch_key, &self.request_hash]
                .iter()
                .any(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
            || uuid::Uuid::parse_str(&self.lease_attempt_id).is_err()
            || self.assignment_id.as_ref().is_some_and(|s| {
                s.len() != 28
                    || !s.starts_with("asg-")
                    || !s[4..].bytes().all(|b| b.is_ascii_hexdigit())
            })
            || self
                .worker_id
                .as_ref()
                .is_some_and(|s| uuid::Uuid::parse_str(s).is_err())
        {
            return Err("native_sdk_log_owner_invalid".into());
        }
        self.owner_id = stable_hash(&json!([self.domain, self.dispatch_key]).to_string());
        Ok(())
    }
    pub(super) fn verify_pristine(&self, db: &Connection) -> Result<(), String> {
        if self.domain == Domain::MapperLoop {
            return super::owner_mapper_loop::verify(db, self, true);
        }
        let next = if self.domain == Domain::Root {
            Self::root(db, &self.run_id, self.round)
                .or_else(|_| super::owner_coordinator::load(db, &self.run_id, self.round))?
        } else {
            let table = if self.domain == Domain::SourceRound {
                "agent_source_model_rounds"
            } else {
                "agent_specialist_calls"
            };
            let round_sql = if self.domain == Domain::SourceRound {
                "c.round_number=?6"
            } else {
                "1=?6"
            };
            let sql=format!("SELECT EXISTS(SELECT 1 FROM {table} c JOIN agent_assignment_attempts x ON x.child_run_id=c.child_run_id
     JOIN agent_runs r ON r.id=c.child_run_id AND r.root_run_id=c.root_run_id AND r.assignment_id=c.assignment_id
     WHERE c.assignment_id=?1 AND c.child_run_id=?2 AND c.root_run_id=?3 AND c.request_hash=?4
       AND x.id=?5 AND {round_sql} AND x.worker_id=?7 AND x.assignment_id=c.assignment_id AND x.root_run_id=c.root_run_id
       AND x.coordinator_epoch=c.lease_epoch AND x.coordinator_fencing_token=c.fencing_token AND c.state='executing'
       AND c.response_json='{{}}' AND c.usage_json='{{}}' AND c.response_hash='' AND c.event_sequence=0 AND c.failure_code=''
       AND r.scan_id=?8 AND r.attempt_number=?9 AND r.role=c.role)");
            let exact: bool = db
                .query_row(
                    &sql,
                    params![
                        self.assignment_id,
                        self.run_id,
                        self.root_run_id,
                        self.request_hash,
                        self.lease_attempt_id,
                        self.round,
                        self.worker_id,
                        self.scan_id,
                        self.attempt
                    ],
                    |r| r.get(0),
                )
                .map_err(|_| "native_sdk_log_original_dispatch_missing")?;
            if !exact {
                return Err("native_sdk_log_original_dispatch_missing".into());
            }
            self.clone()
        };
        if next != *self {
            return Err("native_sdk_log_original_binding_changed".into());
        }
        Ok(())
    }
    pub(super) fn cost_phase(&self, db: &Connection) -> Result<Option<String>, String> {
        if self.domain == Domain::MapperLoop {
            return super::owner_mapper_loop::cost_phase(db, self);
        }
        if self.domain == Domain::Root {
            return db
                .query_row(
                    "SELECT phase FROM agent_root_model_journal WHERE call_id=?1 AND root_run_id=?2
      AND lease_attempt_id=?3 AND request_hash=?4 AND round=?5 AND phase<>'dispatch'",
                    params![
                        self.dispatch_key,
                        self.root_run_id,
                        self.lease_attempt_id,
                        self.request_hash,
                        self.round
                    ],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|_| "native_sdk_log_cost_read_failed".into());
        }
        let family = if self.domain == Domain::SourceRound {
            "source-round"
        } else {
            "specialist"
        };
        let late:Option<String>=db.query_row("SELECT phase FROM agent_model_cost_facts WHERE family=?1 AND root_run_id=?2
    AND assignment_id=?3 AND lease_attempt_id=?4 AND child_run_id=?5 AND round_number=?6 AND request_hash=?7",
    params![family,self.root_run_id,self.assignment_id,self.lease_attempt_id,self.run_id,self.round,self.request_hash],|r|r.get(0)).optional().map_err(|_|"native_sdk_log_cost_read_failed")?;
        if late.is_some() {
            return Ok(late);
        }
        let table = if self.domain == Domain::SourceRound {
            "agent_source_model_rounds"
        } else {
            "agent_specialist_calls"
        };
        let round_sql = if self.domain == Domain::SourceRound {
            "round_number=?4"
        } else {
            "1=?4"
        };
        let sql=format!("SELECT state,failure_code,response_hash FROM {table} WHERE child_run_id=?1 AND root_run_id=?2 AND request_hash=?3 AND {round_sql}");
        let row: Option<(String, String, String)> = db
            .query_row(
                &sql,
                params![self.run_id, self.root_run_id, self.request_hash, self.round],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|_| "native_sdk_log_cost_read_failed")?;
        let Some((state, code, hash)) = row else {
            return Ok(None);
        };
        if code == "model_cancelled_before_transport" {
            return Ok(Some("unsent".into()));
        }
        let source = if family == "specialist" {
            format!(
                "specialist:{}:{}",
                self.assignment_id.as_deref().unwrap_or_default(),
                self.request_hash
            )
        } else {
            format!(
                "source-round:{}:{}:{}",
                self.assignment_id.as_deref().unwrap_or_default(),
                self.round,
                self.request_hash
            )
        };
        let paid_source = format!("{source}:receipt:{hash}");
        let fact:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2
    AND lease_attempt_id=?3 AND dimension='model_requests' AND ((?4='received' AND kind='consume' AND amount=1 AND source_id=?5)
      OR (?4='uncertain' AND kind='forfeit' AND amount=1 AND source_id=?6)))",params![self.root_run_id,self.assignment_id,self.lease_attempt_id,state,paid_source,source],|r|r.get(0)).map_err(|_|"native_sdk_log_cost_read_failed")?;
        Ok((fact && matches!(state.as_str(), "received" | "uncertain")).then_some(state))
    }
}
