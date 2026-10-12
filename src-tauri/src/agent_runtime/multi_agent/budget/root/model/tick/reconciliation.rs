//! Explicit Root cost reconciliation after a committed v3 invoice.
//! Settles unsettled received bills. Forfeited holds can become consumed only
//! for the exact reported cost inside frozen caps, never for a dispatch estimate.
//! Never resends the provider SDK or enlarges frozen caps.
use super::{unpublished_projection::invoice_response, Tick};
use crate::agent_runtime::multi_agent::budget::{self, Kind, DIMENSIONS};
use rusqlite::{params, OptionalExtension, Transaction};
#[cfg(test)]
use rusqlite::{Connection, TransactionBehavior};
use serde_json::Value;

impl Tick {
    /// Finish cost settlement and terminal proof for a v3 received invoice that
    /// was committed before settlement writers completed. Uses only the frozen
    /// journal invoice — no SDK replay.
    /// Test harness entry; production requires explicit confirmation binding.
    #[cfg(test)]
    pub(crate) fn settle_unsettled_received(
        db: &Connection,
        root: &str,
        call_id: &str,
    ) -> Result<(), String> {
        if !db.is_autocommit() {
            return Err("root_tick_reconcile_requires_own_transaction".into());
        }
        let tx = Transaction::new_unchecked(db, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let (tick, capture) = super::super::super::transaction::protect(&tx, || {
            Self::settle_unsettled_received_in(&tx, root, call_id)
        })?;
        tx.commit().map_err(|e| e.to_string())?;
        tick.persist_terminal_capture(db, capture)
    }
    pub(super) fn settle_unsettled_received_in(
        tx: &Transaction<'_>,
        root: &str,
        call_id: &str,
    ) -> Result<(Self, super::TerminalCapture), String> {
        let tick = Self::original_request_for_projection(tx, root, call_id)?;
        if tick.request_fact["version"] != 3 {
            return Err("root_tick_reconcile_requires_v3".into());
        }
        let proof: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_root_tick_terminal_proofs
                WHERE call_id=?1 AND root_run_id=?2 AND lease_attempt_id=?3
                  AND round=?4 AND request_hash=?5 AND basis_hash=?6 AND phase='received')",
                params![
                    tick.call.id,
                    tick.call.owner.root,
                    tick.call.owner.id,
                    tick.call.round,
                    tick.call.request,
                    tick.basis
                ],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if proof {
            return Err("root_tick_reconcile_already_settled".into());
        }
        let raw: String = tx
            .query_row(
                "SELECT receipt_json FROM agent_root_model_journal
                WHERE call_id=?1 AND root_run_id=?2 AND lease_attempt_id=?3
                  AND round=?4 AND request_hash=?5 AND phase='received'",
                params![
                    tick.call.id,
                    tick.call.owner.root,
                    tick.call.owner.id,
                    tick.call.round,
                    tick.call.request
                ],
                |r| r.get(0),
            )
            .map_err(|_| "root_tick_reconcile_received_missing".to_string())?;
        let invoice: Value = serde_json::from_str(&raw).map_err(|_| "root_tick_json_invalid")?;
        // Compare serialized JSON bytes, not Value's string-value equality.
        let canonical_invoice = invoice.to_string();
        if canonical_invoice != raw {
            return Err("root_tick_json_invalid".into());
        }
        tick.call.verify(tx, "received", &invoice)?;
        let progressed: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_budget_entries
                WHERE root_run_id=?1 AND assignment_id='' AND lease_attempt_id=?2
                  AND source_id=?3 AND kind<>'reserve')",
                params![tick.call.owner.root, tick.call.owner.id, tick.call.id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if progressed {
            return Err("root_tick_reconcile_cost_already_progressed".into());
        }
        let response = invoice_response(&invoice)?;
        super::super::receipt::settle(tx, &tick.call, "received", Some(&response))?;
        tick.call.verify(tx, "received", &invoice)?;
        tick.verify_request(tx)?;
        let capture = tick.capture_terminal_proof(tx, "received", Some(&response));
        Ok((tick, capture))
    }

    /// Consume only exactly bill-backed forfeits for a terminal v3 invoice.
    /// Missing/inconsistent usage, over-cap bills and amounts requiring a new
    /// settlement protocol stay unresolved. Never refunds or retries transport.
    /// Test harness entry; production uses the confirmed transaction below.
    #[cfg(test)]
    pub(crate) fn reconcile_forfeited_unknown(
        db: &Connection,
        root: &str,
        call_id: &str,
    ) -> Result<(), String> {
        if !db.is_autocommit() {
            return Err("root_tick_reconcile_requires_own_transaction".into());
        }
        let tx = Transaction::new_unchecked(db, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        super::super::super::transaction::protect(&tx, || {
            Self::reconcile_forfeited_unknown_in(&tx, root, call_id)
        })?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub(super) fn reconcile_forfeited_unknown_in(
        tx: &Transaction<'_>,
        root: &str,
        call_id: &str,
    ) -> Result<(), String> {
        let tick = Self::original_request_for_projection(tx, root, call_id)?;
        if tick.request_fact["version"] != 3 {
            return Err("root_tick_reconcile_requires_v3".into());
        }
        tick.verify_original_terminal_proof(tx)?;
        let raw: String = tx
            .query_row(
                "SELECT receipt_json FROM agent_root_model_journal
            WHERE call_id=?1 AND root_run_id=?2 AND lease_attempt_id=?3
              AND round=?4 AND request_hash=?5 AND phase='received'",
                params![
                    tick.call.id,
                    tick.call.owner.root,
                    tick.call.owner.id,
                    tick.call.round,
                    tick.call.request
                ],
                |r| r.get(0),
            )
            .map_err(|_| "root_tick_reconcile_received_missing".to_string())?;
        let invoice: Value = serde_json::from_str(&raw).map_err(|_| "root_tick_json_invalid")?;
        tick.call.verify(tx, "received", &invoice)?;
        let response = invoice_response(&invoice)?;
        let mut open = false;
        for dimension in DIMENSIONS {
            if budget::balance(tx, root, None, dimension)?.indeterminate > 0 {
                open = true;
                break;
            }
        }
        if !open {
            let done: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM agent_budget_entries
                WHERE root_run_id=?1 AND assignment_id='' AND lease_attempt_id=?2
                  AND source_id=?3 AND kind='reconcile')",
                    params![tick.call.owner.root, tick.call.owner.id, tick.call.id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            return if done {
                Err("root_tick_reconcile_already_done".into())
            } else if !tick.cost_unknown(tx, &response)? {
                Err("root_tick_reconcile_not_unknown".into())
            } else {
                Err("root_tick_reconcile_forfeit_missing".into())
            };
        }
        if !tick.cost_unknown(tx, &response)? {
            return Err("root_tick_reconcile_not_unknown".into());
        }
        // Original proof records a forfeit, not the actual fee. A human
        // confirmation cannot turn an estimate into a verified invoice.
        // Reuse the received-cost validator for complete provider usage
        // and the original shared/per-dimension ceilings before any write.
        if !tick.call.known_cost_before_settlement(tx, &response)? {
            return Err("root_tick_reconcile_actual_cost_unverified".into());
        }
        let mut any = false;
        for (dimension, actual) in DIMENSIONS[..3].iter().zip([
            response.usage.input_tokens,
            response.usage.cached_input_tokens,
            response.usage.output_tokens,
        ]) {
            let amount: Option<i64> = tx
                .query_row(
                    "SELECT amount FROM agent_budget_entries
                WHERE root_run_id=?1 AND assignment_id='' AND lease_attempt_id=?2
                  AND source_id=?3 AND dimension=?4 AND kind='forfeit'",
                    params![
                        tick.call.owner.root,
                        tick.call.owner.id,
                        tick.call.id,
                        dimension
                    ],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            let Some(amount) = amount.filter(|n| *n > 0) else {
                return Err("root_tick_reconcile_forfeit_missing".into());
            };
            if amount != actual {
                return Err("root_tick_reconcile_actual_cost_mismatch".into());
            }
            any = true;
            tick.call.owner.append(
                tx,
                dimension,
                Kind::Reconcile,
                amount,
                &format!("root-model:{}:{dimension}:reconcile", tick.call.id),
                &tick.call.id,
            )?;
        }
        if !any {
            return Err("root_tick_reconcile_forfeit_missing".into());
        }
        tick.verify_request(tx)?;
        tick.call.verify(tx, "received", &invoice)?;
        tick.verify_original_terminal_proof(tx)?;
        // Class limits alone do not prove the shared input+output ceiling.
        super::super::super::shared::available(tx, root)?;
        for dimension in DIMENSIONS {
            if budget::balance(tx, root, None, dimension)?.indeterminate > 0 {
                return Err("root_tick_reconcile_indeterminate_remaining".into());
            }
        }
        Ok(())
    }
}
