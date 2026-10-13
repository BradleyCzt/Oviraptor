//! Terminal signal reduction (§15).
//!
//! This module computes outcomes from `TerminalSignals`. Production persistence
//! is enforced by the single-projection and child lifecycle/recovery writers;
//! the test-only `commit` helper is not their shared production write gate.
pub(crate) mod backend_exit;
use super::contract::{terminal_code, TerminalSignals, TerminalState};
#[cfg(test)]
use super::store;
#[cfg(test)]
use rusqlite::Connection;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reduction {
    pub state: TerminalState,
    pub code: String,
    pub reason: String,
}

impl Reduction {
    // Phase 2/3 consumes this; the rules around it are covered by tests now.
    #[allow(dead_code)]
    pub fn as_json(&self) -> serde_json::Value {
        serde_json::json!({
            "terminal": self.state.as_str(),
            "code": self.code,
            "reason": self.reason,
        })
    }
}

/// Resolution order is fixed: a user cancel beats a configuration error, which
/// beats a protection boundary, which beats budget and capability questions.
/// "Nothing found" is never a failure, and `Incomplete` is never produced just
/// because `confirmed_findings == 0`.
pub fn reduce(signals: &TerminalSignals) -> Reduction {
    let fallback_reason = signals.detail.clone();
    if signals.cancelled {
        return Reduction {
            state: TerminalState::Cancelled,
            code: terminal_code::USER_CANCELLED.to_string(),
            reason: non_empty(&fallback_reason, "用户已取消当前目标"),
        };
    }
    if let Some(error) = &signals.configuration_error {
        return Reduction {
            state: TerminalState::Failed,
            code: if error.contains("完整性") || error.contains("integrity") {
                terminal_code::EVIDENCE_INTEGRITY.to_string()
            } else {
                terminal_code::CONFIGURATION.to_string()
            },
            reason: error.clone(),
        };
    }
    // Phase 2 §5.2: a failed local write ends the attempt on its own terms, before
    // anything else can be blamed for it.
    if let Some(reason) = &signals.persistence_failure {
        return Reduction {
            state: TerminalState::PersistenceFailure,
            code: terminal_code::PERSISTENCE_FAILURE.to_string(),
            reason: reason.clone(),
        };
    }
    // Evidence and a closed coverage ledger cannot settle an unknown request or
    // replace execution authorization. These stops are not automatic retries.
    if let Some(reason) = &signals.request_reconciliation_required {
        return Reduction {
            state: TerminalState::Incomplete,
            code: terminal_code::REQUEST_RECONCILIATION_REQUIRED.to_string(),
            reason: reason.clone(),
        };
    }
    if let Some(reason) = &signals.execution_authorization_denied {
        return Reduction {
            state: TerminalState::Incomplete,
            code: terminal_code::EXECUTION_AUTHORIZATION_DENIED.to_string(),
            reason: reason.clone(),
        };
    }
    // A continuation that cannot inherit its parent is its own end state: the user
    // has to re-run explicitly, and it must not read as a model or tool failure.
    if let Some(reason) = &signals.resume_incompatible {
        return Reduction {
            state: TerminalState::ResumeIncompatible,
            code: terminal_code::RESUME_INCOMPATIBLE.to_string(),
            reason: reason.clone(),
        };
    }
    if let Some(signal) = &signals.protection_signal {
        let rate_limited = signal.contains("429") || signal.contains("限流");
        return Reduction {
            state: TerminalState::Limited,
            code: if rate_limited {
                terminal_code::PERSISTENT_RATE_LIMIT.to_string()
            } else {
                terminal_code::CONFIRMED_CHALLENGE.to_string()
            },
            reason: signal.clone(),
        };
    }
    if let Some(reason) = &signals.unsupported_capability {
        return Reduction {
            state: TerminalState::Incomplete,
            code: terminal_code::UNSUPPORTED_CAPABILITY.to_string(),
            reason: reason.clone(),
        };
    }
    if let Some(reason) = &signals.hard_limit_reason {
        return Reduction {
            state: if signals.evidence_records > 0 {
                TerminalState::BoundedCompleted
            } else {
                TerminalState::Incomplete
            },
            code: if reason.to_ascii_lowercase().contains("token") {
                terminal_code::HARD_TOKEN_BUDGET.to_string()
            } else {
                terminal_code::HARD_REQUEST_BUDGET.to_string()
            },
            reason: reason.clone(),
        };
    }
    if let Some(reason) = &signals.soft_budget_stall_reason {
        return Reduction {
            state: if signals.evidence_records > 0 {
                TerminalState::BoundedCompleted
            } else {
                TerminalState::Incomplete
            },
            code: if reason.to_ascii_lowercase().contains("token") {
                terminal_code::SOFT_BUDGET_STALLED.to_string()
            } else {
                terminal_code::NO_PROGRESS_WINDOW.to_string()
            },
            reason: reason.clone(),
        };
    }
    if signals.ledger_closed {
        // §10 (Phase 2) / §11: a closed ledger that still names gaps is a bounded
        // completion, never a plain "completed". A close-out may also leave
        // queued contracts for a later/manual pass; those are visible gaps even
        // when every family in the submitted ledger was covered or inapplicable.
        let gaps = signals.uncovered_families();
        if !gaps.is_empty() || signals.pending_contracts > 0 {
            return Reduction {
                state: if signals.evidence_records > 0 {
                    TerminalState::BoundedCompleted
                } else {
                    TerminalState::Incomplete
                },
                code: terminal_code::LEDGER_COMPLETE.to_string(),
                reason: non_empty(
                    &fallback_reason,
                    &format!(
                        "覆盖账本已收口：已覆盖 {} 个族，仍有 {} 个族未取得证据、{} 项合同待后续处理",
                        signals.covered_families.len(),
                        gaps.len(),
                        signals.pending_contracts
                    ),
                ),
            };
        }
        return Reduction {
            state: TerminalState::Completed,
            code: terminal_code::FINISH_TARGET.to_string(),
            reason: non_empty(
                &fallback_reason,
                &format!(
                    "覆盖账本已收口：已覆盖 {} 个族",
                    signals.covered_families.len()
                ),
            ),
        };
    }
    // No ledger close and no stop signal: the run only counts as complete when
    // nothing is left pending; otherwise it stays recoverable work.
    if signals.pending_contracts > 0 {
        return Reduction {
            state: TerminalState::Incomplete,
            code: terminal_code::LEDGER_COMPLETE.to_string(),
            reason: non_empty(
                &fallback_reason,
                &format!("仍有 {} 项合同未形成可用结果", signals.pending_contracts),
            ),
        };
    }
    Reduction {
        state: TerminalState::Completed,
        code: terminal_code::LEDGER_COMPLETE.to_string(),
        reason: non_empty(&fallback_reason, "计划内合同均已形成可用结果"),
    }
}

fn non_empty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

/// Reduce and persist. A run that already holds a terminal state keeps it — the
/// second writer is rejected rather than allowed to rewrite history.
#[cfg(test)]
pub fn commit(
    connection: &Connection,
    run_id: &str,
    signals: &TerminalSignals,
) -> Result<Reduction, String> {
    let reduction = reduce(signals);
    // First writer wins: a run that already holds a terminal state is never
    // rewritten, whatever a later backend or retry reports.
    if let Some(existing) = store::load_run(connection, run_id)? {
        if existing.is_terminal() {
            return Ok(Reduction {
                state: existing.terminal_state.unwrap_or(TerminalState::Failed),
                code: existing.terminal_code.clone(),
                reason: existing.terminal_reason.clone(),
            });
        }
    }
    store::mark_run_terminal(
        connection,
        run_id,
        reduction.state,
        &reduction.code,
        &reduction.reason,
    )?;
    Ok(reduction)
}
