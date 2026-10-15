use crate::{asset_models::CollectionSummary, db};
use rusqlite::params;
use serde_json::Value;
use std::{fs, path::Path};

fn count(value: &Value, name: &str) -> u64 {
    value.get(name).and_then(Value::as_u64).unwrap_or(0)
}

fn codes(value: &Value, name: &str) -> Vec<String> {
    let mut result = value
        .get(name)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 80
                && s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
        .take(32)
        .map(str::to_string)
        .collect::<Vec<_>>();
    result.sort();
    result.dedup();
    result
}

pub(super) fn binding(value: &Value) -> Option<&str> {
    value
        .get("binding_sha256")
        .and_then(Value::as_str)
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Coverage is a proof about every planned query, not the execution's success flag.
/// Missing fields and contradictory receipts must remain unknown.
pub(super) fn proven_complete(value: &Value) -> bool {
    let Some(queries) = value
        .get("queries")
        .and_then(Value::as_u64)
        .filter(|n| *n > 0)
    else {
        return false;
    };
    value.get("complete").and_then(Value::as_bool) == Some(true)
        && value.get("coverage_complete").and_then(Value::as_bool) == Some(true)
        && value.get("completed_queries").and_then(Value::as_u64) == Some(queries)
        && [
            "failed_queries",
            "truncated_queries",
            "partial_queries",
            "invalid_records",
        ]
        .iter()
        .all(|key| value.get(key).and_then(Value::as_u64) == Some(0))
        && value
            .get("field_coverage_complete")
            .and_then(Value::as_bool)
            == Some(true)
        && value
            .get("coverage_reasons")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
        && value
            .get("error_categories")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
}

pub(super) fn from_collection(value: &Value) -> CollectionSummary {
    let execution_complete = value.get("complete").and_then(Value::as_bool) == Some(true);
    let coverage_complete = proven_complete(value);
    let mut reasons = codes(value, "coverage_reasons");
    if !coverage_complete && reasons.is_empty() {
        reasons.push("coverage_unproven".into());
    }
    CollectionSummary {
        schema_version: 1,
        execution_complete,
        coverage_complete,
        queries: count(value, "queries"),
        completed_queries: count(value, "completed_queries"),
        failed_queries: count(value, "failed_queries"),
        truncated_queries: count(value, "truncated_queries"),
        partial_queries: count(value, "partial_queries"),
        invalid_records: value
            .get("invalid_records")
            .or_else(|| value.get("rejected_rows"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        candidates: count(value, "candidates"),
        imported_candidates: Some(0),
        probed_assets: None,
        reused_pages: count(value, "reused_pages"),
        error_categories: codes(value, "error_categories"),
        reasons,
        retention: None,
    }
}

pub(super) fn retention(
    report: &super::collection_retention::RetentionReport,
    limits: super::collection_retention::Limits,
) -> crate::asset_models::CollectionRetentionSummary {
    crate::asset_models::CollectionRetentionSummary {
        scope: "owned_collection_files_and_control_metadata_only".into(),
        run_bytes: report.run_bytes,
        project_bytes: report.project_bytes,
        global_bytes: report.global_bytes,
        // The UI's retention scope is all registered collection directories.
        // Count removals there, including another expired run reclaimed now.
        expired_files: report.global_expired_files,
        expired_bytes: report.global_expired_bytes,
        unmanaged_runs: report.unmanaged_runs as u64,
        unknown_files: report.unknown_files as u64,
        history_truncated: report.history_truncated,
        max_run_bytes: limits.run_bytes,
        max_project_bytes: limits.project_bytes,
        max_global_bytes: limits.global_bytes,
        max_pages: limits.pages as u64,
        max_age_seconds: limits.age_seconds as u64,
    }
}

/// Only accept the collector's bounded atomic receipt, never infer coverage
/// from a file's presence, candidate count, error text or progress percentage.
pub(super) fn failed_receipt(directory: &Path) -> CollectionSummary {
    let path = directory.join("collection_state.json");
    let receipt = fs::metadata(&path)
        .ok()
        .filter(|m| m.len() <= 8 * 1024 * 1024)
        .and_then(|_| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .unwrap_or(Value::Null);
    let mut summary = from_collection(&receipt);
    summary.execution_complete = false;
    summary.coverage_complete = false;
    if !summary
        .reasons
        .iter()
        .any(|code| code == "collection_failed")
    {
        summary.reasons.push("collection_failed".into());
    }
    summary
}

pub(super) fn save_on(
    connection: &rusqlite::Connection,
    run_id: i64,
    summary: &CollectionSummary,
) -> Result<(), String> {
    let encoded = serde_json::to_string(summary).map_err(|e| e.to_string())?;
    if encoded.len() > 16 * 1024 {
        return Err("资产收集摘要超出保存上限".into());
    }
    let changed = connection
        .execute(
            "UPDATE runs SET collection_summary_json=?1 WHERE id=?2",
            params![encoded, run_id],
        )
        .map_err(|e| e.to_string())?;
    if changed != 1 {
        return Err("资产收集摘要对应任务不存在".into());
    }
    Ok(())
}

pub(super) fn save(db_path: &Path, run_id: i64, summary: &CollectionSummary) -> Result<(), String> {
    save_on(&db::open(db_path)?, run_id, summary)
}

pub(super) fn probe_count(value: &Value) -> Option<u64> {
    value
        .get("stages")?
        .as_object()?
        .values()
        .try_fold(0u64, |sum, stage| {
            if stage.get("complete").and_then(Value::as_bool) != Some(true) {
                return None;
            }
            let counts = stage.get("counts")?.as_object()?;
            let selected = stage.get("selected_rows")?.as_u64()?;
            let total = counts
                .values()
                .try_fold(0u64, |n, v| n.checked_add(v.as_u64()?))?;
            if total != selected {
                return None;
            }
            sum.checked_add(total.checked_sub(counts.get("skipped")?.as_u64()?)?)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn partial_coverage_is_independent_of_successful_execution_and_persists() {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE runs(id INTEGER PRIMARY KEY, collection_summary_json TEXT NOT NULL DEFAULT '');INSERT INTO runs(id) VALUES(7);").unwrap();
        let mut summary = from_collection(
            &json!({"complete":true,"coverage_complete":false,"queries":3,"completed_queries":3,"truncated_queries":1,"candidates":12,"coverage_reasons":["page_limit"]}),
        );
        summary.imported_candidates = Some(12);
        save_on(&connection, 7, &summary).unwrap();
        let encoded: String = connection
            .query_row("SELECT collection_summary_json FROM runs", [], |r| r.get(0))
            .unwrap();
        let actual: CollectionSummary = serde_json::from_str(&encoded).unwrap();
        assert!(actual.execution_complete);
        assert!(!actual.coverage_complete);
        assert_eq!(actual.truncated_queries, 1);
        assert_eq!(actual.imported_candidates, Some(12));
        assert_eq!(actual.reasons, ["page_limit"]);
        assert!(save_on(&connection, 99, &summary).is_err());
    }

    #[test]
    fn malformed_or_failed_receipts_never_claim_coverage_or_imported_assets() {
        let summary = from_collection(
            &json!({"complete":false,"coverage_complete":true,"failed_queries":1,"candidates":8,"error_categories":["provider_quota", "credential=value"]}),
        );
        assert!(!summary.coverage_complete);
        assert_eq!(summary.imported_candidates, Some(0));
        assert_eq!(summary.probed_assets, None);
        assert_eq!(summary.error_categories, ["provider_quota"]);
        let summary = failed_receipt(Path::new("/definitely-missing-oviraptor-fixture"));
        assert!(!summary.execution_complete && !summary.coverage_complete);
    }

    #[test]
    fn retention_view_uses_registered_scope_and_global_removal_counters_without_breaking_old_receipts(
    ) {
        let mut report = super::super::collection_retention::RetentionReport::default();
        report.expired_files = 1;
        report.global_expired_files = 7;
        report.global_expired_bytes = 1024;
        report.unmanaged_runs = 2;
        let view = retention(
            &report,
            super::super::collection_retention::Limits::default(),
        );
        assert_eq!(
            view.scope,
            "owned_collection_files_and_control_metadata_only"
        );
        assert_eq!(view.expired_files, 7);
        assert_eq!(view.expired_bytes, 1024);
        assert_eq!(view.unmanaged_runs, 2);
        assert_eq!(view.max_global_bytes, 2 * 1024 * 1024 * 1024);
        let mut old = serde_json::to_value(from_collection(&json!({}))).unwrap();
        old.as_object_mut().unwrap().remove("retention");
        assert!(serde_json::from_value::<CollectionSummary>(old)
            .unwrap()
            .retention
            .is_none());
    }

    #[test]
    fn probe_counts_require_actual_complete_stage_receipts() {
        assert_eq!(
            probe_count(
                &json!({"stages":{"a":{"complete":true,"selected_rows":3,"counts":{"web_alive":2,"skipped":1}},"b":{"complete":true,"selected_rows":2,"counts":{"unreachable":2,"skipped":0}}}})
            ),
            Some(4)
        );
        assert_eq!(
            probe_count(&json!({"stages":{"a":{"complete":false,"selected_rows":3}}})),
            None
        );
        assert_eq!(
            probe_count(&json!({"stages":{"a":{"complete":true}}})),
            None
        );
        assert_eq!(
            probe_count(
                &json!({"stages":{"a":{"complete":true,"selected_rows":3,"counts":{"web_alive":2,"skipped":0}}}})
            ),
            None
        );
        assert_eq!(probe_count(&json!({})), None);
    }

    #[test]
    fn coverage_requires_a_complete_nonempty_consistent_query_proof() {
        let complete = json!({"complete":true,"coverage_complete":true,"queries":2,
            "completed_queries":2,"failed_queries":0,"truncated_queries":0,
            "partial_queries":0,"invalid_records":0,"field_coverage_complete":true,
            "coverage_reasons":[],"error_categories":[]});
        assert!(from_collection(&complete).coverage_complete);
        assert!(
            !from_collection(&json!({"complete":true,"coverage_complete":true})).coverage_complete
        );
        for (key, value) in [
            ("complete", json!(false)),
            ("queries", json!(0)),
            ("completed_queries", json!(3)),
            ("failed_queries", json!(1)),
            ("truncated_queries", json!(1)),
            ("partial_queries", json!(1)),
            ("invalid_records", json!(1)),
            ("field_coverage_complete", json!(false)),
            ("coverage_reasons", json!(["page_limit"])),
            ("error_categories", json!(["provider_quota"])),
        ] {
            let mut receipt = complete.clone();
            receipt[key] = value;
            assert!(!proven_complete(&receipt), "contradiction accepted: {key}");
        }
        for key in [
            "queries",
            "completed_queries",
            "failed_queries",
            "field_coverage_complete",
            "error_categories",
        ] {
            let mut receipt = complete.clone();
            receipt.as_object_mut().unwrap().remove(key);
            assert!(!proven_complete(&receipt), "missing proof accepted: {key}");
        }
        assert!(binding(&json!({"binding_sha256":"ab".repeat(32)})).is_some());
        assert!(binding(&json!({"binding_sha256":"query1"})).is_none());
    }

    #[test]
    fn failed_collection_receipt_preserves_partial_counts_without_claiming_import() {
        let root = std::env::temp_dir().join(format!(
            "oviraptor-summary-fixture-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("collection_state.json"),
            json!({
                "complete":false,"coverage_complete":false,"queries":3,"completed_queries":2,
                "failed_queries":1,"partial_queries":1,"candidates":7,"reused_pages":2,
                "error_categories":["provider_quota"],"coverage_reasons":["provider_error"],
            })
            .to_string(),
        )
        .unwrap();
        let summary = failed_receipt(&root);
        assert_eq!(
            (
                summary.queries,
                summary.completed_queries,
                summary.failed_queries,
                summary.candidates
            ),
            (3, 2, 1, 7)
        );
        assert_eq!(summary.imported_candidates, Some(0));
        assert_eq!(summary.probed_assets, None);
        assert_eq!(summary.error_categories, ["provider_quota"]);
        assert!(!summary.coverage_complete && !summary.execution_complete);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn historical_runs_gain_unknown_summary_without_reinterpreting_state() {
        let root = std::env::temp_dir().join(format!(
            "oviraptor-summary-migration-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = db::initialize(&root).unwrap();
        let connection = db::open(&path).unwrap();
        connection.execute_batch("INSERT INTO projects(id,name) VALUES(917,'synthetic summary migration');INSERT INTO runs(id,project_id,name,status,stage,progress,error) VALUES(918,917,'legacy','completed','completed',100,'old evidence');ALTER TABLE runs DROP COLUMN collection_summary_json;").unwrap();
        drop(connection);
        for _ in 0..2 {
            db::initialize(&root).unwrap();
            let connection = db::open(&path).unwrap();
            let state: (String,String,f64,String,String) = connection.query_row("SELECT status,stage,progress,error,collection_summary_json FROM runs WHERE id=918",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
            assert_eq!(
                state,
                (
                    "completed".into(),
                    "completed".into(),
                    100.0,
                    "old evidence".into(),
                    String::new()
                )
            );
            assert!(serde_json::from_str::<CollectionSummary>(&state.4).is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn historical_membership_scope_migration_is_unknown_and_preserves_manual_state() {
        let root = std::env::temp_dir().join(format!(
            "oviraptor-binding-migration-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = db::initialize(&root).unwrap();
        let connection = db::open(&path).unwrap();
        connection.execute_batch("INSERT INTO projects(id,name) VALUES(917,'synthetic scope migration');INSERT INTO assets(id,asset_key,company) VALUES(919,'synthetic-migration-key','fixture');INSERT INTO project_assets(project_id,asset_id,decision,note,is_deleted,last_seen) VALUES(917,919,'confirmed','retained manual decision',1,'legacy unknown');ALTER TABLE project_assets DROP COLUMN last_collection_binding;").unwrap();
        drop(connection);
        for _ in 0..2 {
            db::initialize(&root).unwrap();
            let connection = db::open(&path).unwrap();
            let state: (String,String,i64,String,String) = connection.query_row("SELECT decision,note,is_deleted,last_seen,last_collection_binding FROM project_assets WHERE project_id=917 AND asset_id=919",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
            assert_eq!(
                state,
                (
                    "confirmed".into(),
                    "retained manual decision".into(),
                    1,
                    "legacy unknown".into(),
                    String::new()
                )
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
}
