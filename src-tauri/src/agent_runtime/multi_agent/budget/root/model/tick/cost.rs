//! Exact original financial write set including physical row identities.
use super::Tick;
use crate::agent_runtime::model::gateway::ModelResponse;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
impl Tick {
    pub(super) fn cost_unknown(&self, db: &Connection, r: &ModelResponse) -> Result<bool, String> {
        if self.request_fact["version"] == 3 {
            return self
                .call
                .original_actual_cost_known(db, r)
                .map(|known| !known);
        }
        let limit:Option<i64>=db.query_row("SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension='model_input_tokens'",[&self.call.owner.root],|r|r.get(0)).map_err(|e|e.to_string())?;
        let u = &r.usage;
        Ok(!(r.usage_reported
            && [
                u.input_tokens,
                u.cached_input_tokens,
                u.output_tokens,
                u.model_requests,
            ]
            .iter()
            .all(|n| *n >= 0)
            && u.model_requests == 1
            && u.cached_input_tokens <= u.input_tokens
            && u.input_tokens.checked_add(u.output_tokens) == Some(u.total_tokens)
            && u.total_tokens >= 0
            && (limit.is_none() || u.total_tokens <= self.call.tokens)))
    }
    pub(super) fn cost_rows(
        &self,
        db: &Connection,
        response: Option<&ModelResponse>,
    ) -> Result<Value, String> {
        use std::collections::BTreeMap;
        let prefix = format!("root:{}:root-model:{}:", self.call.owner.id, self.call.id);
        let mut expected = BTreeMap::new();
        let values = response.map(|r| {
            [
                r.usage.input_tokens,
                r.usage.cached_input_tokens,
                r.usage.output_tokens,
                r.usage.model_requests,
            ]
        });
        let known = match response {
            Some(r) => !self.cost_unknown(db, r)?,
            None => false,
        };
        let finite_actual = self.request_fact["version"] == 3
            && db.query_row("SELECT hard_limit IS NOT NULL FROM agent_budget_limits WHERE root_run_id=?1 AND dimension='model_input_tokens'", [&self.call.owner.root], |r| r.get::<_, bool>(0)).map_err(|e|e.to_string())?;
        for (index, dimension) in crate::agent_runtime::multi_agent::budget::DIMENSIONS[..4]
            .iter()
            .enumerate()
        {
            let reserve = if index == 3 { 1 } else { self.call.tokens };
            expected.insert(
                format!("{prefix}{dimension}:reserve"),
                (dimension.to_string(), "reserve".to_string(), reserve),
            );
            if let Some(values) = values {
                let held = if known && index < 3 {
                    reserve.max(values[index])
                } else {
                    reserve
                };
                if held > reserve {
                    expected.insert(
                        format!(
                            "{prefix}{dimension}:{}",
                            if finite_actual { "actual" } else { "unlimited" }
                        ),
                        (dimension.to_string(), "reserve".into(), held - reserve),
                    );
                }
                let (kind, amount) = if index == 3 {
                    ("consume", 1)
                } else if known {
                    ("consume", values[index])
                } else {
                    ("forfeit", held)
                };
                if amount > 0 {
                    expected.insert(
                        format!("{prefix}{dimension}:terminal"),
                        (dimension.to_string(), kind.into(), amount),
                    );
                }
                if (index == 3 || known) && held > amount {
                    expected.insert(
                        format!("{prefix}{dimension}:release"),
                        (dimension.to_string(), "release".into(), held - amount),
                    );
                }
            }
        }
        let mut stmt=db.prepare("SELECT rowid,entry_id,root_run_id,assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key,source_id,created_at
            FROM agent_budget_entries WHERE (source_id=?1 OR idempotency_key LIKE ?2)
              AND kind<>'reconcile'
              AND (?3=0 OR substr(idempotency_key,-8)=':reserve') ORDER BY entry_id").map_err(|e|e.to_string())?;
        let rows=stmt.query_map(params![self.call.id,format!("{prefix}%"),response.is_none()],|r|Ok(json!({
            "rowid":r.get::<_,i64>(0)?,"id":r.get::<_,String>(1)?,"root":r.get::<_,String>(2)?,"assignment":r.get::<_,String>(3)?,
            "owner":r.get::<_,String>(4)?,"dimension":r.get::<_,String>(5)?,"kind":r.get::<_,String>(6)?,"amount":r.get::<_,i64>(7)?,
            "key":r.get::<_,String>(8)?,"source":r.get::<_,String>(9)?,"created":r.get::<_,String>(10)?,
        }))).map_err(|e|e.to_string())?.collect::<Result<Vec<Value>,_>>().map_err(|e|e.to_string())?;
        for row in &rows {
            let key = row["key"]
                .as_str()
                .ok_or("root_tick_original_cost_changed")?;
            let tuple = expected
                .remove(key)
                .ok_or("root_tick_original_cost_changed")?;
            if row["root"] != self.call.owner.root
                || row["assignment"] != ""
                || row["owner"] != self.call.owner.id
                || row["source"] != self.call.id
                || row["dimension"] != tuple.0
                || row["kind"] != tuple.1
                || row["amount"] != tuple.2
                || row["rowid"].as_i64().is_none_or(|n| n <= 0)
                || row["id"]
                    .as_str()
                    .is_none_or(|s| uuid::Uuid::parse_str(s).is_err())
            {
                return Err("root_tick_original_cost_changed".into());
            }
        }
        if !expected.is_empty() {
            return Err("root_tick_original_cost_missing".into());
        }
        Ok(Value::Array(rows))
    }
}
