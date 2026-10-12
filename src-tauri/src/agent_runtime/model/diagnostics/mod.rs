//! Safe SDK stages, independent of process streams, model bytes and accounting.
mod owner;
mod owner_coordinator;
mod owner_mapper_loop;
mod owner_saved;
mod persist;
pub(crate) mod replay;
mod replay_metadata;
mod transitions;
use crate::agent_runtime::multi_agent::{lease::CoordinatorLease, scheduler::ScheduledChild};
pub(crate) use persist::Hint;
use rusqlite::Connection;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
#[derive(Clone, Copy)]
pub(crate) enum TransportStage {
    Sent,
    ResponseReceived,
}
pub(crate) type StageObserver = Arc<dyn Fn(TransportStage) + Send + Sync>;
struct State {
    db: Connection,
    owner: owner::Owner,
    cost: bool,
    finished: bool,
    failed: bool,
}
impl State {
    fn stage(&mut self, stage: &str, cost: &str, terminal: &str) {
        if self.failed || self.finished {
            return;
        }
        match persist::append(&mut self.db, &self.owner, stage, cost, terminal) {
            Ok(hint) => {
                let _ = crate::commands::notify_native_sdk_log(&hint);
            }
            Err(_) => {
                self.failed = true;
                if let Ok(hint) = persist::gap(&mut self.db, &self.owner, stage) {
                    let _ = crate::commands::notify_native_sdk_log(&hint);
                }
            }
        }
    }
    fn cost(&mut self) -> Option<String> {
        let phase = self.owner.cost_phase(&self.db).ok().flatten();
        if let Some(phase) = &phase {
            if !self.cost {
                self.stage("cost_saved", phase, "");
                self.cost = true;
            }
        }
        phase
    }
    fn finish(&mut self, returned: bool) {
        if self.finished {
            return;
        }
        let cost = self.cost();
        let validated = returned && cost.as_deref() == Some("received");
        if validated {
            self.stage("validated", "", "");
        }
        let state = if validated {
            "returned"
        } else {
            match cost.as_deref() {
                Some("received") => "withheld",
                Some("uncertain") => "uncertain",
                Some("unsent") => "unsent",
                _ => "failed",
            }
        };
        self.stage("terminal", cost.as_deref().unwrap_or_default(), state);
        self.finished = true;
    }
}
pub(crate) struct ModelLog {
    state: Option<Arc<Mutex<State>>>,
}
impl ModelLog {
    fn begin(path: &Path, load: impl FnOnce(&Connection) -> Result<owner::Owner, String>) -> Self {
        let state = (|| {
            let mut db = persist::connection(path)?;
            let owner = load(&db)?;
            let prepared = persist::begin(&mut db, &owner)?;
            let _ = crate::commands::notify_native_sdk_log(&prepared);
            let state = State {
                db,
                owner,
                cost: false,
                finished: false,
                failed: false,
            };
            Ok::<_, String>(Arc::new(Mutex::new(state)))
        })()
        .ok();
        Self { state }
    }
    pub(crate) fn root(path: &Path, run: &str, round: i64) -> Self {
        Self::begin(path, |db| owner::Owner::root(db, run, round))
    }
    pub(crate) fn coordinator(path: &Path, run: &str, round: i64) -> Self {
        Self::begin(path, |db| owner_coordinator::load(db, run, round))
    }
    pub(crate) fn child(
        path: &Path,
        lease: &CoordinatorLease,
        child: &ScheduledChild,
        round: i64,
        tool: bool,
    ) -> Self {
        Self::begin(path, |db| {
            owner::Owner::child(db, lease, child, round, tool)
        })
    }
    pub(crate) fn mapper_loop(
        path: &Path,
        lease: &CoordinatorLease,
        child: &ScheduledChild,
        round: i64,
    ) -> Self {
        Self::begin(path, |db| owner_mapper_loop::load(db, lease, child, round))
    }
    pub(crate) fn observer(&self) -> StageObserver {
        let state = self.state.clone();
        Arc::new(move |stage| {
            if let Some(state) = &state {
                if let Ok(mut state) = state.lock() {
                    state.stage(
                        match stage {
                            TransportStage::Sent => "sent",
                            TransportStage::ResponseReceived => "response_received",
                        },
                        "",
                        "",
                    );
                }
            }
        })
    }
    pub(crate) fn cost_saved(&self) {
        if let Some(state) = &self.state {
            if let Ok(mut state) = state.lock() {
                state.cost();
            }
        }
    }
    pub(crate) fn finish(&self, returned: bool) {
        if let Some(state) = &self.state {
            if let Ok(mut state) = state.lock() {
                state.finish(returned);
            }
        }
    }
}
impl Drop for ModelLog {
    fn drop(&mut self) {
        self.finish(false);
    }
}

#[cfg(test)]
pub(crate) mod contract_tests;
