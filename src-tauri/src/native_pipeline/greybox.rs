//! §10.3 — greybox: the source branch and the browser branch are two ways of filling
//! *one* evidence graph, so a conflict between them is an edge the reviewer can see, not
//! a reason to drop either side or to fall back to another engine.

use super::snapshot::RepositorySnapshot;
use crate::agent_runtime::contract::AgentRole;
use crate::agent_runtime::evidence_graph::contract::{
    EvidenceEdge, EvidenceEdgeKind, EvidenceNode, EvidenceNodeKind, EvidenceProvenance,
};
use crate::agent_runtime::evidence_graph::store::{
    evidence_natural_key, insert_evidence_edge, insert_evidence_node,
    list_evidence_edges_at_revision, list_evidence_nodes_at_revision, EvidenceInsert,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value as JsonValue};

/// How many read-only analysis assignments one target may hold at the same time.
/// Stage 4 keeps this at one; raising it is a later stage's decision (§10.3 item 5).
pub const READ_ONLY_LANE_CAPACITY: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Branch {
    Source,
    Runtime,
}

impl Branch {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Runtime => "runtime",
        }
    }
}

/// What a branch ended with. A failed branch always carries at least one gap: "we did
/// not look" has to be readable from the result, not inferred from an absence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchClose {
    pub branch: Branch,
    pub ok: bool,
    pub gaps: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conclusion {
    SourceOnly,
    RuntimeVerified,
    Contradicted,
}

impl Conclusion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SourceOnly => "source_only",
            Self::RuntimeVerified => "runtime_verified",
            Self::Contradicted => "contradicted",
        }
    }

    /// Only a fact the browser actually produced may be called runtime verified.
    pub fn is_runtime_verified(self) -> bool {
        self == Self::RuntimeVerified
    }
}

#[derive(Clone, Debug)]
pub struct GreyboxGraph {
    pub root_run_id: String,
    pub run_id: String,
    pub revision: i64,
    pub target_key: String,
    pub tree_hash: String,
}

impl GreyboxGraph {
    pub fn new(
        root_run_id: &str,
        run_id: &str,
        revision: i64,
        target_key: &str,
        snapshot: &RepositorySnapshot,
    ) -> Self {
        Self {
            root_run_id: root_run_id.to_string(),
            run_id: run_id.to_string(),
            revision,
            target_key: target_key.to_string(),
            tree_hash: snapshot.tree_hash.clone(),
        }
    }

    fn node(
        &self,
        kind: EvidenceNodeKind,
        identity: &str,
        provenance: EvidenceProvenance,
        payload: JsonValue,
    ) -> EvidenceNode {
        let hash = evidence_natural_key(&self.root_run_id, kind, identity);
        EvidenceNode {
            id: format!("{}-{}", kind.as_str(), &hash[..16]),
            root_run_id: self.root_run_id.clone(),
            revision: self.revision,
            kind,
            provenance,
            natural_key_hash: hash,
            payload,
            artifact_refs: vec![self.tree_hash.clone()],
            created_by_run_id: self.run_id.clone(),
            supersedes_id: String::new(),
            created_at: String::new(),
        }
    }

    /// One source-derived claim. Provenance is fixed here: the caller cannot ask for
    /// `observed`, because nothing on this branch touched the target.
    pub fn record_source_claim(
        &self,
        connection: &Connection,
        identity: &str,
        claim: &str,
        path: &str,
        line: u64,
    ) -> Result<String, String> {
        let node = self.node(
            EvidenceNodeKind::CandidateFinding,
            &format!("source|{identity}"),
            EvidenceProvenance::SourceDerived,
            json!({
                "branch": Branch::Source.as_str(),
                "claim": claim,
                "path": path,
                "line": line,
                "treeHash": self.tree_hash,
                "reviewState": "candidate",
            }),
        );
        match insert_evidence_node(connection, &node)? {
            EvidenceInsert::Inserted(id) | EvidenceInsert::Existing(id) => Ok(id),
        }
    }

    /// One runtime observation from the browser/CDP branch.
    pub fn record_runtime_observation(
        &self,
        connection: &Connection,
        identity: &str,
        payload: JsonValue,
    ) -> Result<String, String> {
        let mut row = serde_json::Map::new();
        row.insert(
            "branch".to_string(),
            JsonValue::String(Branch::Runtime.as_str().to_string()),
        );
        if let Some(map) = payload.as_object() {
            row.extend(map.clone());
        }
        let node = self.node(
            EvidenceNodeKind::RequestRecord,
            &format!("runtime|{identity}"),
            EvidenceProvenance::Observed,
            JsonValue::Object(row),
        );
        match insert_evidence_node(connection, &node)? {
            EvidenceInsert::Inserted(id) | EvidenceInsert::Existing(id) => Ok(id),
        }
    }

    /// A runtime fact that supports a source claim. This edge is the only thing that can
    /// turn a conclusion into `runtime_verified`.
    pub fn support(
        &self,
        connection: &Connection,
        runtime_node: &str,
        source_node: &str,
        reason: &str,
    ) -> Result<(), String> {
        self.edge(
            connection,
            runtime_node,
            source_node,
            EvidenceEdgeKind::Supports,
            json!({"reason": reason}),
        )
        .map(|_| ())
    }

    /// §10.3 item 2: the contradiction is recorded as an edge. Both nodes stay, because
    /// deleting the static evidence would destroy the thing a reviewer needs to compare.
    pub fn contradict(
        &self,
        connection: &Connection,
        runtime_node: &str,
        source_node: &str,
        reason: &str,
    ) -> Result<(), String> {
        self.edge(
            connection,
            runtime_node,
            source_node,
            EvidenceEdgeKind::Contradicts,
            json!({"reason": reason, "staticEvidenceKept": true}),
        )?;
        Ok(())
    }

    fn edge(
        &self,
        connection: &Connection,
        from: &str,
        to: &str,
        kind: EvidenceEdgeKind,
        payload: JsonValue,
    ) -> Result<EvidenceInsert, String> {
        insert_evidence_edge(
            connection,
            &EvidenceEdge {
                id: 0,
                root_run_id: self.root_run_id.clone(),
                revision: self.revision,
                from_node_id: from.to_string(),
                to_node_id: to.to_string(),
                kind,
                payload,
                created_by_run_id: self.run_id.clone(),
                created_at: String::new(),
            },
        )
    }

    /// §10.3 item 4: how strong a conclusion may be called, derived from the graph
    /// rather than from what the model asserted.
    pub fn conclusion_of(
        &self,
        connection: &Connection,
        source_node: &str,
    ) -> Result<Conclusion, String> {
        let nodes = list_evidence_nodes_at_revision(connection, &self.root_run_id, self.revision)?;
        let edges = list_evidence_edges_at_revision(connection, &self.root_run_id, self.revision)?;
        let observed = |id: &str| {
            nodes
                .iter()
                .any(|node| node.id == id && node.provenance == EvidenceProvenance::Observed)
        };
        if edges.iter().any(|edge| {
            edge.kind == EvidenceEdgeKind::Contradicts
                && edge.to_node_id == source_node
                && observed(&edge.from_node_id)
        }) {
            return Ok(Conclusion::Contradicted);
        }
        if edges.iter().any(|edge| {
            edge.kind == EvidenceEdgeKind::Supports
                && edge.to_node_id == source_node
                && observed(&edge.from_node_id)
        }) {
            return Ok(Conclusion::RuntimeVerified);
        }
        Ok(Conclusion::SourceOnly)
    }

    /// §10.3 item 3: when the source branch fails, the web branch closes with a gap.
    /// There is no third option here — no other engine is started to fill it in.
    pub fn close_branch(
        &self,
        branch: Branch,
        failure: Option<&str>,
        gaps: &[String],
    ) -> BranchClose {
        let mut merged = gaps.to_vec();
        match (branch, failure) {
            (Branch::Source, Some(detail)) => {
                merged.push(format!("source_branch_failed:{detail}"));
                merged.push("web_branch_closed_with_gap".to_string());
            }
            (Branch::Runtime, Some(detail)) => {
                merged.push(format!("runtime_branch_failed:{detail}"));
            }
            (_, None) => {}
        }
        BranchClose {
            branch,
            ok: failure.is_none(),
            gaps: merged,
        }
    }
}

/// §10.3 item 1 / GRY-002: the key that says "this HTTP endpoint is that controller,
/// symbol and source location". It is built from the frozen tree, so the same commit
/// always maps the same way and a different source file never collides into it.
pub fn route_binding(
    tree_hash: &str,
    method: &str,
    endpoint: &str,
    controller: &str,
    symbol: &str,
    path: &str,
    line: u64,
) -> String {
    let normalized = endpoint
        .split_once(['?', '#'])
        .map_or(endpoint, |(head, _)| head)
        .trim_end_matches('/');
    evidence_natural_key(
        tree_hash,
        EvidenceNodeKind::Endpoint,
        &format!(
            "{}|{}|{}|{}|{path}:{line}",
            method.to_ascii_uppercase(),
            normalized,
            controller,
            symbol
        ),
    )
}

/// Which runs currently hold a read-only analysis assignment for this target.
pub fn read_only_lane_holders(
    connection: &Connection,
    target_key: &str,
) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT DISTINCT child_run_id FROM agent_assignments
             WHERE lane='read_only_analysis' AND target_key=?1
               AND scheduling_model='legacy_role_pipeline'
               AND state NOT IN ('completed','failed','cancelled','rejected')
             ORDER BY child_run_id",
        )
        .map_err(|error| format!("无法检查只读分析通道：{error}"))?;
    let rows = statement
        .query_map([target_key], |row| row.get::<_, String>(0))
        .map_err(|error| format!("无法检查只读分析通道：{error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法读取只读分析通道：{error}"))
}

/// After both greybox branches have a terminal receipt, copy their real facts into one
/// evidence graph. Source claims come from the source report (filled by the importer).
/// Runtime facts come from completed tool invocations of this attempt. A path both
/// sides state is a `supports` edge; two different methods on that path are
/// `contradicts`. Nothing here invents a request or deletes a source node.
pub fn project_closed_attempt(
    connection: &Connection,
    scan_id: &str,
    attempt: i64,
) -> Result<Vec<String>, String> {
    let scan_type: String = connection
        .query_row(
            "SELECT scan_type FROM sentinel_scans WHERE id=?1",
            [scan_id],
            |row| row.get(0),
        )
        .unwrap_or_default();
    if scan_type != "greybox" {
        return Ok(Vec::new());
    }
    let pending: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM native_scan_branches WHERE scan_id=?1 AND attempt_number=?2 AND status='pending'",
            params![scan_id, attempt],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法确认灰盒分支是否都已结束：{error}"))?;
    if pending > 0 {
        return Ok(Vec::new());
    }
    let source_status: String = connection
        .query_row(
            "SELECT status FROM native_scan_branches WHERE scan_id=?1 AND attempt_number=?2 AND branch='source'",
            params![scan_id, attempt],
            |row| row.get(0),
        )
        .unwrap_or_default();
    let source_report: String = connection
        .query_row(
            "SELECT report_json FROM native_scan_branches WHERE scan_id=?1 AND attempt_number=?2 AND branch='source'",
            params![scan_id, attempt],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "{}".into());
    let mut notes = Vec::new();
    if source_status == "failed" {
        let detail = serde_json::from_str::<JsonValue>(&source_report)
            .ok()
            .and_then(|value| {
                value
                    .get("error")
                    .and_then(JsonValue::as_str)
                    .map(str::to_string)
            })
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "source_branch_failed".into());
        let placeholder = GreyboxGraph {
            root_run_id: String::new(),
            run_id: String::new(),
            revision: 1,
            target_key: scan_id.to_string(),
            tree_hash: String::new(),
        };
        notes.extend(
            placeholder
                .close_branch(Branch::Source, Some(&detail), &[])
                .gaps,
        );
    }
    let Some(snapshot) = RepositorySnapshot::restore(connection, scan_id, attempt)? else {
        notes.push("greybox_snapshot_unavailable".into());
        return Ok(notes);
    };
    let run_id = format!("greybox:{scan_id}:{attempt}");
    connection
        .execute(
            "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,backend,role,status) \
             VALUES(?1,?2,?3,'', 'native','coordinator','completed') \
             ON CONFLICT(id) DO NOTHING",
            params![run_id, scan_id, attempt],
        )
        .map_err(|error| format!("无法建立灰盒证据图的 run：{error}"))?;
    let graph = GreyboxGraph::new(&run_id, &run_id, 1, scan_id, &snapshot);
    let report = serde_json::from_str::<JsonValue>(&source_report).unwrap_or(JsonValue::Null);
    let mut source_nodes = Vec::new();
    if let Some(claims) = report.get("sourceClaims").and_then(JsonValue::as_array) {
        for claim in claims {
            let path = text_of(claim, &["path", "endpoint", "repoPath"]);
            if path.is_empty() {
                continue;
            }
            let method = text_of(claim, &["method"]).to_ascii_uppercase();
            let line = claim.get("line").and_then(JsonValue::as_u64).unwrap_or(0);
            let title = text_of(claim, &["claim", "title", "rule"]);
            let identity = if method.is_empty() {
                format!("{path}:{line}")
            } else {
                format!("{method}|{}", normalize_route(&path))
            };
            let id = graph.record_source_claim(connection, &identity, &title, &path, line)?;
            source_nodes.push((method, normalize_route(&path), id));
        }
    }
    let mut statement = connection
        .prepare(
            "SELECT i.input_summary_json,i.progress_signature,i.contract_key \
             FROM tool_invocations i JOIN agent_runs r ON r.id=i.run_id \
             WHERE r.scan_id=?1 AND r.attempt_number=?2 AND i.status='completed' \
               AND r.id<>?3",
        )
        .map_err(|error| format!("无法读取本次尝试的运行时调用：{error}"))?;
    let rows = statement
        .query_map(params![scan_id, attempt, run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| format!("无法读取本次尝试的运行时调用：{error}"))?;
    let mut supports = 0usize;
    let mut contradictions = 0usize;
    for row in rows {
        let (summary, signature, contract) = row.map_err(|error| error.to_string())?;
        let Some((method, path)) = runtime_route(&summary, &signature, &contract) else {
            continue;
        };
        let observed = graph.record_runtime_observation(
            connection,
            &format!("{method}|{path}"),
            json!({"method": method, "path": path}),
        )?;
        for (source_method, source_path, source_id) in &source_nodes {
            if source_path != &path {
                continue;
            }
            if source_method.is_empty() || source_method == &method {
                graph.support(connection, &observed, source_id, "same_route")?;
                supports += 1;
            } else {
                graph.contradict(connection, &observed, source_id, "method_disagrees")?;
                contradictions += 1;
            }
        }
    }
    notes.push(format!(
        "greybox_graph:source={} runtime_supports={supports} contradictions={contradictions}",
        source_nodes.len()
    ));
    Ok(notes)
}

fn text_of(value: &JsonValue, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(JsonValue::as_str))
        .unwrap_or("")
        .trim()
        .to_string()
}

fn normalize_route(path: &str) -> String {
    let bare = path
        .split_once(['?', '#'])
        .map_or(path, |(head, _)| head)
        .trim();
    let without_origin = bare
        .split_once("://")
        .and_then(|(_, rest)| rest.find('/').map(|index| &rest[index..]))
        .unwrap_or(bare);
    without_origin.trim_end_matches('/').to_string()
}

fn runtime_route(summary: &str, signature: &str, contract: &str) -> Option<(String, String)> {
    if let Ok(value) = serde_json::from_str::<JsonValue>(summary) {
        let method = text_of(&value, &["method"]).to_ascii_uppercase();
        let path = normalize_route(&text_of(&value, &["path", "endpoint", "url"]));
        if !method.is_empty() && path.starts_with('/') {
            return Some((method, path));
        }
    }
    for raw in [signature, contract] {
        let mut parts = raw.split([' ', '|']).filter(|part| !part.is_empty());
        let method = parts.next().unwrap_or("").to_ascii_uppercase();
        let path = normalize_route(parts.next().unwrap_or(""));
        if matches!(
            method.as_str(),
            "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD"
        ) && path.starts_with('/')
        {
            return Some((method, path));
        }
    }
    None
}

/// Claim the single read-only slot for this target. Re-claiming it with the same run is
/// a no-op; a second run is refused, which is how §10.3 item 5 stays true.
pub fn claim_read_only_lane(
    connection: &Connection,
    target_key: &str,
    run_id: &str,
) -> Result<(), String> {
    let holders = read_only_lane_holders(connection, target_key)?;
    if holders.iter().any(|holder| holder == run_id) {
        return Ok(());
    }
    if holders.len() >= READ_ONLY_LANE_CAPACITY {
        return Err(format!(
            "目标 {target_key} 的只读分析通道已被 {} 占用，容量只有 {READ_ONLY_LANE_CAPACITY}",
            holders.join(", ")
        ));
    }
    connection
        .execute(
            "INSERT INTO agent_assignments(
                id,coordinator_run_id,child_run_id,role,lane,target_key,state,dedup_key
             ) VALUES(?1,?1,?1,?2,'read_only_analysis',?3,'running',?4)",
            params![
                run_id,
                AgentRole::Coordinator.as_str(),
                target_key,
                format!("read-only:{target_key}"),
            ],
        )
        .map_err(|error| format!("无法占用只读分析通道：{error}"))?;
    Ok(())
}
