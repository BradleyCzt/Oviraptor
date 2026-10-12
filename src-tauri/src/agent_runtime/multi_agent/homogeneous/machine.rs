//! SPEC §5.1 / DESIGN §3.2：机器可检的 Rust 独立重算。模型只能回显，不能改写。

use super::{store, types};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value as JsonValue;

/// SPEC §1.5 / DESIGN §5.5：Worker output 的形状 = 规格合法示例的三个键，不增不减。
pub fn output_shape_valid(output: &JsonValue) -> bool {
    classify_output(output).is_ok()
}

/// SPEC §1.5 非法例 3：output 里凭空出现工具键（`"new_tool":"shell.exec"`）算 invented_tool，
/// 其它形状问题算 output_schema_invalid。
pub fn classify_output(output: &JsonValue) -> Result<(), ()> {
    let Some(map) = output.as_object() else {
        return Err(());
    };
    if map.len() == 3
        && map
            .get("summary_zh")
            .and_then(|value| value.as_str())
            .map(|value| value.chars().count() <= 2000)
            .unwrap_or(false)
        && map
            .get("findings")
            .map(|value| value.is_array())
            .unwrap_or(false)
        && map
            .get("coverage_notes")
            .and_then(|value| value.as_array())
            .map(|items| {
                items.len() <= 64
                    && items
                        .iter()
                        .all(|item| item.is_string() && !item.as_str().unwrap_or("").is_empty())
            })
            .unwrap_or(false)
    {
        return Ok(());
    }
    Err(())
}

pub fn output_invents_tool(output: &JsonValue) -> bool {
    output
        .as_object()
        .map(|map| {
            map.keys().any(|key| {
                key != "summary_zh"
                    && key != "findings"
                    && key != "coverage_notes"
                    && key.contains("tool")
            })
        })
        .unwrap_or(false)
}

fn node_for_alias(
    connection: &Connection,
    alias: &str,
    root_run_id: &str,
) -> Option<(String, i64, JsonValue)> {
    let (internal_id, node_root, _alias_revision) =
        store::resolve_evidence_alias(connection, alias).ok()??;
    if node_root != root_run_id {
        return None;
    }
    let stored: Option<(i64, String)> = connection
        .query_row(
            "SELECT revision,payload_json FROM agent_evidence_nodes WHERE id=?1 AND root_run_id=?2",
            params![internal_id.clone(), root_run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .ok()?;
    let (revision, payload_json) = stored?;
    let parsed: JsonValue = serde_json::from_str(&payload_json).unwrap_or(JsonValue::Null);
    Some((internal_id, revision, parsed))
}

fn payload_path(payload: &JsonValue) -> Option<String> {
    payload
        .get("path")
        .and_then(|value| value.as_str())
        .or_else(|| payload.get("url").and_then(|value| value.as_str()))
        .map(super::decompose::normalize_path_prefix)
}

fn payload_host(payload: &JsonValue) -> Option<String> {
    payload
        .get("host")
        .and_then(|value| value.as_str())
        .or_else(|| payload.get("url").and_then(|value| value.as_str()))
        .map(super::decompose::host_of)
}

pub(super) fn within_scope(slice: &types::TaskSlice, payload: &JsonValue) -> bool {
    match slice.scope_kind.as_str() {
        "path_prefix" => match payload_path(payload) {
            Some(path) => {
                let scope = super::decompose::normalize_path_prefix(&slice.scope_value);
                path == scope || path.starts_with(&format!("{scope}/")) || scope == "/"
            }
            None => true,
        },
        "host" => match payload_host(payload) {
            Some(host) => host == slice.scope_value,
            None => true,
        },
        "asset_id" | "source_type" => {
            match payload.get("asset_id").and_then(|value| value.as_str()) {
                Some(asset) => asset == slice.scope_value,
                None => true,
            }
        }
        _ => false,
    }
}

fn invoked_tools(
    connection: &Connection,
    assignment: &types::HomogeneousAssignment,
) -> Vec<String> {
    if assignment.worker_id.is_empty() {
        return Vec::new();
    }
    connection
        .prepare("SELECT DISTINCT tool_name FROM tool_invocations WHERE run_id=?1")
        .and_then(|mut statement| {
            statement
                .query_map([format!("run-{}", assignment.assignment_id)], |row| {
                    row.get::<_, String>(0)
                })
                .and_then(|rows| rows.collect::<Result<Vec<String>, _>>())
        })
        .unwrap_or_default()
}

/// 同一切片之外是否有人声称了同一证据（SPEC §1.6 no_cross_slice_write）。
pub(super) fn claimed_by_other_slice(
    connection: &Connection,
    parent_task_id: &str,
    slice_id: &str,
    alias: &str,
) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_worker_reports r JOIN agent_task_slices s ON s.slice_id=r.slice_id \
             WHERE s.parent_task_id=?1 AND r.slice_id<>?2 AND r.evidence_refs_json LIKE '%'||?3||'%')",
            params![parent_task_id, slice_id, alias],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| error.to_string())
}

/// SPEC §5.1 的七项重算。任一 false 时禁止 accept，且 model_judgement 必须为 null。
pub fn recompute(
    connection: &Connection,
    task: &types::HomogeneousTask,
    assignment: &types::HomogeneousAssignment,
    slice: &types::TaskSlice,
    report: &types::WorkerReport,
) -> types::MachineChecks {
    let mut scope_in_bounds = true;
    let mut evidence_refs_exist = true;
    let mut evidence_revision_fresh = true;
    let mut no_cross_slice_write = true;
    // SPEC §1.5：evidence_refs 允许为空数组，但「若有结论则非空」。
    // 因此只有当汇报声称了结论（findings / coverage_notes 非空，或自称 completed）时，
    // 空引用才算证据缺失。
    let claims_a_conclusion = report
        .output
        .get("findings")
        .and_then(|value| value.as_array())
        .map(|items| !items.is_empty())
        .unwrap_or(false)
        || report
            .output
            .get("coverage_notes")
            .and_then(|value| value.as_array())
            .map(|items| !items.is_empty())
            .unwrap_or(false)
        || report.terminal_hint == "completed";
    if report.evidence_refs.is_empty() && claims_a_conclusion {
        evidence_refs_exist = false;
    }
    for alias in &report.evidence_refs {
        match node_for_alias(connection, alias, &task.root_run_id) {
            Some((_internal, revision, payload)) => {
                if !within_scope(slice, &payload) {
                    scope_in_bounds = false;
                }
                if revision < slice.evidence_revision_base {
                    evidence_revision_fresh = false;
                }
                if claimed_by_other_slice(connection, &task.task_id, &slice.slice_id, alias)
                    .unwrap_or(true)
                {
                    no_cross_slice_write = false;
                }
            }
            None => evidence_refs_exist = false,
        }
    }
    let budget_ok = report.consumed_tokens <= assignment.reserved_tokens
        && report.consumed_requests <= assignment.reserved_requests;
    let lease_tools: Vec<String> = assignment.capability_lease.clone();
    let tools_authorized = invoked_tools(connection, assignment)
        .into_iter()
        .all(|tool| lease_tools.iter().any(|allowed| allowed == &tool));
    let output_schema_ok = output_shape_valid(&report.output);
    types::MachineChecks {
        scope_in_bounds,
        budget_ok,
        tools_authorized,
        output_schema_ok,
        evidence_refs_exist,
        evidence_revision_fresh,
        no_cross_slice_write,
    }
}

/// SPEC §2.3 校验 2：模型回显与重算逐键比对，不一致即 machine_check_failed。
pub fn matches(payload: &types::MachineChecks, recomputed: &types::MachineChecks) -> bool {
    payload == recomputed
}
