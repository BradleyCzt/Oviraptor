//! §12 Stage 4 — the Native source pipeline: a frozen repository snapshot, bounded
//! analyzers, a read-only tool broker over the snapshot, and the CI/greybox surfaces on
//! top of them. Nothing here can launch a retired external backend, and every missing
//! capability surfaces as a gap instead of a fallback.

// Stage 4 ships the pipeline and the tool surface; the orchestration and UI that call
// every entry point land in later stages, and each one is exercised by this stage's
// tests.
#![allow(dead_code)]

pub mod analysis_view;
pub mod analyzer;
pub mod ci;
mod container;
pub mod greybox;
pub mod process;
pub mod results;
pub mod snapshot;
pub(crate) mod source_ci;
pub mod tools;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value as JsonValue};

use crate::agent_runtime::store::stable_hash;
use snapshot::RepositorySnapshot;

fn snapshot_text(snapshot: &JsonValue, key: &str) -> String {
    snapshot
        .get(key)
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The frozen plan for a source-carrying scan. Like the web plan it is hashed once and
/// inherited verbatim by every continuation; unlike the web plan it names a snapshot and
/// an analyzer set instead of an origin list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeSourcePlan {
    pub scan_id: String,
    pub attempt_number: i64,
    pub scan_type: String,
    pub backend: String,
    pub tree_hash: String,
    pub commit_sha: String,
    pub base_sha: String,
    pub diff_base_state: String,
    pub languages: Vec<String>,
    pub analyzers: Vec<String>,
    pub tools: Vec<String>,
    /// Capabilities this scan asked for but cannot have. A gap is reported, never
    /// silently downgraded to a retired backend (§Stage 4 exit gate).
    pub gaps: Vec<String>,
}

impl NativeSourcePlan {
    pub fn for_snapshot(
        scan_id: &str,
        attempt_number: i64,
        scan_type: &str,
        snapshot: &RepositorySnapshot,
        analyzers: &[&str],
        gaps: Vec<String>,
    ) -> Self {
        let mut merged = snapshot.gaps.clone();
        merged.extend(gaps);
        merged.sort();
        merged.dedup();
        Self {
            scan_id: scan_id.to_string(),
            attempt_number,
            scan_type: scan_type.to_string(),
            backend: "native".to_string(),
            tree_hash: snapshot.tree_hash.clone(),
            commit_sha: snapshot.commit_sha.clone(),
            base_sha: snapshot.base_sha.clone(),
            diff_base_state: snapshot.diff_base.as_str().to_string(),
            languages: snapshot.languages(),
            analyzers: analyzers.iter().map(|value| value.to_string()).collect(),
            tools: tools::SOURCE_TOOLS
                .iter()
                .map(|value| value.to_string())
                .collect(),
            gaps: merged,
        }
    }

    pub fn as_json(&self) -> JsonValue {
        json!({
            "schemaVersion": 1,
            "owner": "oviraptor",
            "backend": self.backend,
            "scanType": self.scan_type,
            "scanId": self.scan_id,
            "attemptNumber": self.attempt_number,
            "snapshot": {
                "treeHash": self.tree_hash,
                "commitSha": self.commit_sha,
                "baseSha": self.base_sha,
                "diffBase": self.diff_base_state,
                "languages": self.languages,
            },
            "analyzers": self.analyzers,
            "allowedTools": self.tools,
            "gaps": self.gaps,
        })
    }

    /// A plan with no analyzers and no files is not a passing scan.
    pub fn is_incomplete(&self) -> bool {
        !self.gaps.is_empty() || self.analyzers.is_empty()
    }

    /// The hash frozen for this attempt. The attempt number is left out for the same
    /// reason the web plan leaves it out: a continuation must inherit the parent's plan
    /// verbatim instead of re-deciding it.
    pub fn hash(&self) -> String {
        let mut row = self.as_json();
        if let Some(map) = row.as_object_mut() {
            map.remove("attemptNumber");
        }
        stable_hash(&row.to_string())
    }

    /// Identify this protocol in the shared frozen-plan table without applying
    /// the Web wire hash to Source's continuation-stable hash. The immutable
    /// scope receipt selects the owner; mutable JSON selectors cannot do so.
    /// This historical read grants no runtime or plaintext capability.
    pub(crate) fn verifies_source_owner(
        connection: &Connection,
        scan_id: &str,
        attempt_number: i64,
        value: &JsonValue,
        hash: &str,
    ) -> Result<bool, String> {
        let (scan_type, backend, source_type): (String, String, Option<String>) = connection
            .query_row(
                "SELECT p.scan_type,p.backend,s.scan_type FROM native_scan_plans p
                 LEFT JOIN source_scope_contracts s ON s.scan_id=p.scan_id AND s.attempt_number=p.attempt_number
                 WHERE p.scan_id=?1 AND p.attempt_number=?2",
                params![scan_id, attempt_number],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|error| format!("source_plan_owner_lookup:{error}"))?;
        let source_shape = value["owner"] == "oviraptor" || value.get("snapshot").is_some();
        if source_type.is_none() && !source_shape {
            return Ok(false);
        }
        let invalid = "source_plan_owner_invalid";
        let plan = Self::from_json(value).ok_or(invalid)?;
        if plan.as_json() != *value
            || plan.scan_id != scan_id
            || plan.attempt_number != attempt_number
            || plan.scan_type != scan_type
            || plan.backend != backend
            || backend != "native"
            || source_type.as_deref() != Some(plan.scan_type.as_str())
            || plan.hash() != hash
        {
            return Err(invalid.into());
        }
        Ok(true)
    }

    pub fn from_json(value: &JsonValue) -> Option<Self> {
        let text = |keys: &[&str]| -> String {
            keys.iter()
                .filter_map(|key| value.get(*key))
                .find_map(JsonValue::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let strings = |value: Option<&JsonValue>| -> Vec<String> {
            value
                .and_then(JsonValue::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter_map(JsonValue::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default()
        };
        let snapshot = value.get("snapshot")?;
        Some(Self {
            scan_id: text(&["scanId"]),
            attempt_number: value
                .get("attemptNumber")
                .and_then(JsonValue::as_i64)
                .unwrap_or(1),
            scan_type: text(&["scanType"]),
            backend: text(&["backend"]),
            tree_hash: snapshot_text(snapshot, "treeHash"),
            commit_sha: snapshot_text(snapshot, "commitSha"),
            base_sha: snapshot_text(snapshot, "baseSha"),
            diff_base_state: snapshot_text(snapshot, "diffBase"),
            languages: strings(snapshot.get("languages")),
            analyzers: strings(value.get("analyzers")),
            tools: strings(value.get("allowedTools")),
            gaps: strings(value.get("gaps")),
        })
    }

    /// Write the freeze. A second, different plan for the same attempt is refused: the
    /// whole point of a frozen plan is that the attempt cannot change its mind.
    pub fn store(&self, connection: &Connection) -> Result<(), String> {
        let hash = self.hash();
        let stored: Option<String> = connection
            .query_row(
                "SELECT plan_hash FROM native_scan_plans WHERE scan_id=?1 AND attempt_number=?2",
                params![self.scan_id, self.attempt_number],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("无法读取冻结计划：{error}"))?;
        if let Some(existing) = stored {
            return if existing == hash {
                Ok(())
            } else {
                Err(format!(
                    "尝试 {}#{} 已经冻结过计划，不能改写（{existing} 对 {hash}）",
                    self.scan_id, self.attempt_number
                ))
            };
        }
        connection
            .execute(
                "INSERT INTO native_scan_plans(scan_id,attempt_number,scan_type,backend,plan_json,plan_hash)
                 VALUES(?1,?2,?3,?4,?5,?6)",
                rusqlite::params![
                    self.scan_id,
                    self.attempt_number,
                    self.scan_type,
                    self.backend,
                    self.as_json().to_string(),
                    hash
                ],
            )
            .map_err(|error| format!("无法冻结计划：{error}"))?;
        Ok(())
    }

    pub fn load(
        connection: &Connection,
        scan_id: &str,
        attempt_number: i64,
    ) -> Result<Option<Self>, String> {
        let stored: Option<(String, String)> = connection
            .query_row(
                "SELECT plan_json,plan_hash FROM native_scan_plans WHERE scan_id=?1 AND attempt_number=?2",
                rusqlite::params![scan_id, attempt_number],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| format!("无法读取冻结计划：{error}"))?;
        let Some((text, hash)) = stored else {
            return Ok(None);
        };
        let parsed: JsonValue = serde_json::from_str(&text)
            .map_err(|error| format!("冻结计划不可解析（{scan_id}#{attempt_number}）：{error}"))?;
        let plan = Self::from_json(&parsed).ok_or_else(|| {
            format!("冻结计划缺少必要字段（{scan_id}#{attempt_number}），已拒绝沿用")
        })?;
        if plan.hash() != hash {
            return Err(format!(
                "冻结计划的哈希与内容不一致（{scan_id}#{attempt_number}），已拒绝沿用"
            ));
        }
        Ok(Some(plan))
    }
}

#[cfg(test)]
mod tests;
