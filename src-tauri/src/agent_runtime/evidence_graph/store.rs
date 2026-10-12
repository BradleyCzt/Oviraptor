//! Persistence for the shared evidence graph (§5.3–§5.4, §6).
//!
//! Node identity is `(root_run_id, revision, natural_key_hash)`, so a replay of the
//! same observation is a no-op rather than a duplicate fact. An edge is refused when
//! an endpoint is missing or belongs to another root or a non-ancestor revision:
//! a dangling or cross-threaded relation would look like evidence later (§9.1). Replays compare
//! content — a same-key row with different bytes is a conflict, never a silent hit.

// The live scheduler, child runs and review gate consume this store. Some
// repository APIs remain acceptance-test-only, so the reachability warning is expected.
#![allow(dead_code)]
use super::contract::{
    EvidenceEdge, EvidenceEdgeKind, EvidenceNode, EvidenceNodeKind, EvidenceProvenance,
};
use crate::agent_runtime::secrets::redact_json;
use crate::agent_runtime::store::{decode_json, stable_hash};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value as JsonValue;

const NODE_COLUMNS: &str = "id,root_run_id,revision,kind,provenance,natural_key_hash,payload_json,\
     artifact_refs_json,created_by_run_id,supersedes_id,created_at";
const EDGE_COLUMNS: &str = "id,root_run_id,revision,from_node_id,to_node_id,kind,payload_json,\
     created_by_run_id,created_at";
const NODE_WHERE: &str = "root_run_id=?1 AND revision=?2 AND natural_key_hash=?3";
const NODE_CONFLICT: &str = "root_run_id,revision,natural_key_hash";
const EDGE_KEY: &str =
    "root_run_id=?1 AND revision=?2 AND from_node_id=?3 AND to_node_id=?4 AND kind=?5";

/// Content-addressed natural key: same root, kind and identifying text always
/// produce the same hash, so a replay deduplicates instead of adding a second fact.
pub fn evidence_natural_key(root_run_id: &str, kind: EvidenceNodeKind, identity: &str) -> String {
    stable_hash(&format!("{root_run_id}|{}|{identity}", kind.as_str()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvidenceInsert {
    Inserted(String),
    /// The same node already exists at this revision.
    Existing(String),
}

type StoredNode = (EvidenceNode, String, String, String, String);
type StoredEdge = (EvidenceEdge, String, String);

fn provenance_strength(provenance: EvidenceProvenance) -> u8 {
    match provenance {
        EvidenceProvenance::Inferred => 0,
        EvidenceProvenance::SourceDerived => 1,
        EvidenceProvenance::Observed => 2,
    }
}

fn node_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredNode> {
    // The enum words and the JSON texts come back untouched; `decode_node` decides
    // whether they are still legible (§3.2, §3.3).
    let node = EvidenceNode {
        id: row.get(0)?,
        root_run_id: row.get(1)?,
        revision: row.get(2)?,
        kind: EvidenceNodeKind::Target,
        provenance: EvidenceProvenance::Inferred,
        natural_key_hash: row.get(5)?,
        payload: JsonValue::Null,
        artifact_refs: Vec::new(),
        created_by_run_id: row.get(8)?,
        supersedes_id: row.get(9)?,
        created_at: row.get(10)?,
    };
    Ok((node, row.get(3)?, row.get(4)?, row.get(6)?, row.get(7)?))
}

fn decode_node(stored: StoredNode) -> Result<EvidenceNode, String> {
    let (mut node, kind, provenance, payload, refs) = stored;
    let id = node.id.clone();
    node.kind = EvidenceNodeKind::try_parse(&kind).ok_or_else(|| {
        format!("表 agent_evidence_nodes 的 kind 不是已知词汇（记录 {id}）：{kind}")
    })?;
    node.provenance = EvidenceProvenance::try_parse(&provenance).ok_or_else(|| {
        format!("表 agent_evidence_nodes 的 provenance 不是已知词汇（记录 {id}）：{provenance}")
    })?;
    node.payload = decode_json("payload_json", "agent_evidence_nodes", &id, &payload)?;
    node.artifact_refs = decode_json("artifact_refs_json", "agent_evidence_nodes", &id, &refs)?;
    Ok(node)
}

fn edge_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredEdge> {
    let edge = EvidenceEdge {
        id: row.get(0)?,
        root_run_id: row.get(1)?,
        revision: row.get(2)?,
        from_node_id: row.get(3)?,
        to_node_id: row.get(4)?,
        kind: EvidenceEdgeKind::DerivedFrom,
        payload: JsonValue::Null,
        created_by_run_id: row.get(7)?,
        created_at: row.get(8)?,
    };
    Ok((edge, row.get(5)?, row.get(6)?))
}

fn decode_edge(stored: StoredEdge) -> Result<EvidenceEdge, String> {
    let (mut edge, kind, payload) = stored;
    let id = edge.id.to_string();
    edge.kind = EvidenceEdgeKind::try_parse(&kind).ok_or_else(|| {
        format!("表 agent_evidence_edges 的 kind 不是已知词汇（记录 {id}）：{kind}")
    })?;
    edge.payload = decode_json("payload_json", "agent_evidence_edges", &id, &payload)?;
    Ok(edge)
}

/// Every fact on the shared graph is attributable: no anonymous author, and the author
/// must be a run that exists (§9.1, §10).
fn require_author(
    connection: &Connection,
    run_id: &str,
    noun: &str,
    id: &str,
) -> Result<(), String> {
    if run_id.trim().is_empty() {
        return Err(format!(
            "evidence {noun} {id} 没有 created_by_run_id，图上事实必须署名"
        ));
    }
    let found: Option<String> = connection
        .query_row("SELECT id FROM agent_runs WHERE id=?1", [run_id], |row| {
            row.get(0)
        })
        .optional()
        .map_err(|error| format!("无法检查 evidence 作者：{error}"))?;
    if found.is_none() {
        return Err(format!(
            "evidence {noun} {id} 的 created_by_run_id 指向不存在的 run：{run_id}"
        ));
    }
    Ok(())
}

/// A supersession is a later version of the same kind in the same root;
/// otherwise it can silently retire another investigation's evidence (§9.1).
fn require_supersedes(connection: &Connection, node: &EvidenceNode) -> Result<(), String> {
    if node.supersedes_id.trim().is_empty() {
        return Ok(());
    }
    let owner: Option<(String, i64, String, String)> = connection
        .query_row(
            "SELECT root_run_id,revision,kind,provenance FROM agent_evidence_nodes WHERE id=?1",
            [&node.supersedes_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|error| format!("无法检查被取代的 evidence node：{error}"))?;
    match owner {
        None => Err(format!(
            "evidence node {} 取代了不存在的节点：{}",
            node.id, node.supersedes_id
        )),
        Some((root, _, _, _)) if root != node.root_run_id => Err(format!(
            "evidence node {} 不能取代其它 root 的节点：{}（{root}）",
            node.id, node.supersedes_id
        )),
        Some((_, revision, _, _)) if node.revision <= revision => Err(format!(
            "evidence node {} 必须以更高 revision 取代节点：{}（{revision}）",
            node.id, node.supersedes_id
        )),
        Some((_, _, kind, _)) if kind != node.kind.as_str() => Err(format!(
            "evidence node {} 不能以不同 kind 取代节点：{}（{kind}）",
            node.id, node.supersedes_id
        )),
        Some((_, predecessor_revision, _, predecessor_provenance)) => {
            let predecessor_provenance = EvidenceProvenance::try_parse(&predecessor_provenance)
                .ok_or_else(|| {
                    format!(
                        "evidence_supersession_unknown_provenance：{}",
                        node.supersedes_id
                    )
                })?;
            if provenance_strength(node.provenance) < provenance_strength(predecessor_provenance) {
                return Err(format!(
                    "evidence_supersession_provenance_downgrade：{} -> {}",
                    node.supersedes_id, node.id
                ));
            }
            let mut ancestors =
                evidence_ancestor_revisions(connection, &node.root_run_id, node.revision)?;
            if ancestors.is_empty() {
                // A new revision is created atomically by the node INSERT
                // trigger. Before it exists, its parent will be the greatest
                // previously registered revision below it.
                let parent: Option<i64> = connection
                    .query_row(
                        "SELECT MAX(revision) FROM agent_evidence_revisions
                     WHERE root_run_id=?1 AND revision<?2",
                        params![node.root_run_id, node.revision],
                        |row| row.get(0),
                    )
                    .map_err(|error| format!("无法检查 evidence revision 父节点：{error}"))?;
                if let Some(parent) = parent {
                    ancestors = evidence_ancestor_revisions(connection, &node.root_run_id, parent)?;
                }
            }
            if !ancestors.contains(&predecessor_revision) {
                return Err(format!(
                    "evidence node {} 不能取代非祖先 revision 的节点：{}",
                    node.id, node.supersedes_id
                ));
            }
            Ok(())
        }
    }
}

fn query_one<T, F>(
    connection: &Connection,
    sql: &str,
    bind: &[&dyn rusqlite::ToSql],
    decode: F,
    noun: &str,
) -> Result<Option<T>, String>
where
    F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    connection
        .query_row(sql, bind, decode)
        .optional()
        .map_err(|error| format!("无法读取 {noun}：{error}"))
}

/// The bytes that make a node what it is. Two nodes with the same natural key and a
/// different value here are a plan drift, not a replay (§3.5).
fn node_content(node: &EvidenceNode, plaintext: bool) -> JsonValue {
    let value=serde_json::json!({
        "kind": node.kind.as_str(),
        "provenance": node.provenance.as_str(),
        "payload": node.payload,
        "artifactRefs": node.artifact_refs,
        "createdByRunId": node.created_by_run_id,
        "supersedesId": node.supersedes_id,
    });
    if plaintext {value} else {redact_json(&value)}
}

fn edge_content(payload: &JsonValue, created_by_run_id: &str) -> JsonValue {
    redact_json(&serde_json::json!({
        "payload": payload,
        "createdByRunId": created_by_run_id,
    }))
}

/// Store one node. The payload and the artifact references go through the runtime's
/// existing redaction before they touch SQL — there is no second scrubber here.
pub fn insert_evidence_node(
    connection: &Connection,
    node: &EvidenceNode,
) -> Result<EvidenceInsert, String> {
    if node.id.trim().is_empty() {
        return Err("evidence node 缺少 id".to_string());
    }
    if node.natural_key_hash.trim().is_empty() {
        return Err("evidence node 缺少 natural_key_hash".to_string());
    }
    if node.root_run_id.trim().is_empty() {
        return Err("evidence node 缺少 root_run_id".to_string());
    }
    if node.revision <= 0 {
        return Err("evidence node revision 必须为正数".to_string());
    }
    let plaintext=crate::agent_runtime::multi_agent::homogeneous::material_policy::for_root(connection,&node.root_run_id)?;
    let payload = if plaintext {node.payload.clone()} else {redact_json(&node.payload)}.to_string();
    let refs = if plaintext {serde_json::json!(node.artifact_refs)} else {redact_json(&serde_json::json!(node.artifact_refs))}.to_string();
    require_author(connection, &node.created_by_run_id, "node", &node.id)?;
    require_supersedes(connection, node)?;
    let owner: Option<(String, i64, String)> = query_one(
        connection,
        "SELECT root_run_id,revision,natural_key_hash FROM agent_evidence_nodes WHERE id=?1",
        &[&node.id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        "evidence node 归属",
    )?;
    if let Some((root, revision, natural_key)) = owner {
        if root != node.root_run_id
            || revision != node.revision
            || natural_key != node.natural_key_hash
        {
            return Err(format!(
                "evidence node id 已被另一个自然键占用，拒绝写入：{}（已属于 {root}/{revision}/{natural_key}）",
                node.id
            ));
        }
    }
    let inserted = connection
        .execute(
            &format!(
                "INSERT INTO agent_evidence_nodes(id,root_run_id,revision,kind,provenance,natural_key_hash,\
                 payload_json,artifact_refs_json,created_by_run_id,supersedes_id,created_at) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,datetime('now','localtime')) \
                 ON CONFLICT({NODE_CONFLICT}) DO NOTHING",
            ),
            params![
                node.id,
                node.root_run_id,
                node.revision,
                node.kind.as_str(),
                node.provenance.as_str(),
                node.natural_key_hash,
                payload,
                refs,
                node.created_by_run_id,
                node.supersedes_id
            ],
        )
        .map_err(|error| format!("无法写入 evidence node：{error}"))?;
    if inserted == 1 {
        return Ok(EvidenceInsert::Inserted(node.id.clone()));
    }
    let stored: Option<StoredNode> = query_one(
        connection,
        &format!("SELECT {NODE_COLUMNS} FROM agent_evidence_nodes WHERE {NODE_WHERE}"),
        &[&node.root_run_id, &node.revision, &node.natural_key_hash],
        node_from_row,
        "重放的 evidence node",
    )?;
    let existing = decode_node(stored.ok_or_else(|| {
        format!(
            "evidence node 未写入且找不到同自然键的记录：{}/{}/{}",
            node.root_run_id, node.revision, node.natural_key_hash
        )
    })?)?;
    if node_content(&existing,plaintext) == node_content(node,plaintext) {
        return Ok(EvidenceInsert::Existing(existing.id));
    }
    Err(format!(
        "evidence_node_conflict：同一自然键已存在不同内容，拒绝覆盖：{}",
        node.natural_key_hash
    ))
}

pub fn load_evidence_node(
    connection: &Connection,
    id: &str,
) -> Result<Option<EvidenceNode>, String> {
    query_one(
        connection,
        &format!("SELECT {NODE_COLUMNS} FROM agent_evidence_nodes WHERE id=?1"),
        &[&id],
        node_from_row,
        "evidence node",
    )?
    .map(decode_node)
    .transpose()
}

pub fn list_evidence_nodes_at_revision(
    connection: &Connection,
    root_run_id: &str,
    revision: i64,
) -> Result<Vec<EvidenceNode>, String> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT {NODE_COLUMNS} FROM agent_evidence_nodes WHERE root_run_id=?1 AND revision=?2 ORDER BY created_at,id"
        ))
        .map_err(|error| format!("无法准备 evidence node 查询：{error}"))?;
    let rows = statement
        .query_map(params![root_run_id, revision], node_from_row)
        .map_err(|error| format!("无法读取 evidence node 列表：{error}"))?;
    rows.map(|item| {
        item.map_err(|error| format!("无法解析 evidence node 列表：{error}"))
            .and_then(decode_node)
    })
    .collect()
}

fn evidence_ancestor_revisions(
    connection: &Connection,
    root_run_id: &str,
    revision: i64,
) -> Result<Vec<i64>, String> {
    let mut statement = connection
        .prepare(
            "WITH RECURSIVE ancestors(revision) AS (
             SELECT revision FROM agent_evidence_revisions
             WHERE root_run_id=?1 AND revision=?2
             UNION ALL
             SELECT r.parent_revision FROM agent_evidence_revisions r
             JOIN ancestors a ON r.root_run_id=?1 AND r.revision=a.revision
             WHERE r.parent_revision IS NOT NULL
         ) SELECT revision FROM ancestors",
        )
        .map_err(|error| format!("无法准备 evidence revision 祖先查询：{error}"))?;
    let rows = statement
        .query_map(params![root_run_id, revision], |row| row.get(0))
        .map_err(|error| format!("无法读取 evidence revision 祖先：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法解析 evidence revision 祖先：{error}"))?;
    Ok(rows)
}

/// Used by Reviewer's verified-fact retirement as well as edge persistence:
/// numeric revision order alone does not prove lineage after a late import.
pub fn evidence_revision_is_ancestor(
    connection: &Connection,
    root_run_id: &str,
    descendant: i64,
    ancestor: i64,
) -> Result<bool, String> {
    Ok(evidence_ancestor_revisions(connection, root_run_id, descendant)?.contains(&ancestor))
}

/// A versioned read keeps facts from the ancestry chain until an attributable,
/// later node version replaces them. No row from a future or unrelated root is
/// visible. An unknown revision is not an alias for the latest snapshot.
pub fn list_effective_evidence_nodes_at_revision(
    connection: &Connection,
    root_run_id: &str,
    revision: i64,
) -> Result<Vec<EvidenceNode>, String> {
    if evidence_ancestor_revisions(connection, root_run_id, revision)?.is_empty() {
        return Ok(Vec::new());
    }
    let mut statement = connection
        .prepare(&format!(
            "WITH RECURSIVE ancestors(revision) AS (
             SELECT revision FROM agent_evidence_revisions
             WHERE root_run_id=?1 AND revision=?2
             UNION ALL
             SELECT r.parent_revision FROM agent_evidence_revisions r
             JOIN ancestors a ON r.root_run_id=?1 AND r.revision=a.revision
             WHERE r.parent_revision IS NOT NULL
         ) SELECT {NODE_COLUMNS} FROM agent_evidence_nodes n
         WHERE n.root_run_id=?1
           AND EXISTS(SELECT 1 FROM ancestors a WHERE a.revision=n.revision)
         ORDER BY n.revision,n.created_at,n.id"
        ))
        .map_err(|error| format!("无法准备 evidence 快照：{error}"))?;
    let nodes: Vec<EvidenceNode> = statement
        .query_map(params![root_run_id, revision], node_from_row)
        .map_err(|error| format!("无法读取 evidence 快照：{error}"))?
        .map(|item| {
            item.map_err(|error| format!("无法解析 evidence 快照：{error}"))
                .and_then(decode_node)
        })
        .collect::<Result<_, _>>()?;
    let by_id: std::collections::HashMap<&str, &EvidenceNode> =
        nodes.iter().map(|node| (node.id.as_str(), node)).collect();
    let mut retired = std::collections::HashSet::new();
    for node in &nodes {
        if node.supersedes_id.is_empty() {
            continue;
        }
        let Some(previous) = by_id.get(node.supersedes_id.as_str()) else {
            return Err(format!(
                "evidence_snapshot_invalid_supersession：{}",
                node.id
            ));
        };
        if previous.revision >= node.revision
            || previous.kind != node.kind
            || provenance_strength(node.provenance) < provenance_strength(previous.provenance)
            || !retired.insert(node.supersedes_id.clone())
        {
            return Err(format!(
                "evidence_snapshot_invalid_supersession：{}",
                node.id
            ));
        }
    }
    Ok(nodes
        .into_iter()
        .filter(|node| !retired.contains(node.id.as_str()))
        .collect())
}

/// Store one relation. Both endpoints must exist in the same root and ancestry: a
/// hypothesis that points at another task's fact would otherwise be quoted as
/// support later (§9.1). The returned id is always the stored rowid.
pub fn insert_evidence_edge(
    connection: &Connection,
    edge: &EvidenceEdge,
) -> Result<EvidenceInsert, String> {
    let ancestors = evidence_ancestor_revisions(connection, &edge.root_run_id, edge.revision)?;
    if ancestors.is_empty() {
        return Err("evidence 边的 revision 不存在".into());
    }
    let mut statement = connection
        .prepare("SELECT id,root_run_id,revision FROM agent_evidence_nodes WHERE id IN (?1,?2)")
        .map_err(|error| format!("无法准备 evidence 端点查询：{error}"))?;
    let endpoints = statement
        .query_map(params![edge.from_node_id, edge.to_node_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(|error| format!("无法检查 evidence 端点：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法读取 evidence 端点：{error}"))?;
    for (side, id) in [
        ("from_node_id", edge.from_node_id.as_str()),
        ("to_node_id", edge.to_node_id.as_str()),
    ] {
        let Some((_, root, revision)) = endpoints.iter().find(|(node, _, _)| node == id) else {
            return Err(format!("evidence 边的{side}不存在，已拒绝写入：{id}"));
        };
        if root != &edge.root_run_id || !ancestors.contains(revision) {
            return Err(format!(
                "evidence 边的{side}不属于本 root/祖先 revision，已拒绝写入：{id}（{root}/{}，期望 {}/{}）",
                revision, edge.root_run_id, edge.revision
            ));
        }
    }
    require_author(
        connection,
        &edge.created_by_run_id,
        "edge",
        &edge.id.to_string(),
    )?;
    let payload = redact_json(&edge.payload);
    let payload_text = payload.to_string();
    let inserted = connection
        .execute(
            "INSERT INTO agent_evidence_edges(root_run_id,revision,from_node_id,to_node_id,kind,payload_json,\
             created_by_run_id,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,datetime('now','localtime')) \
             ON CONFLICT(root_run_id,revision,from_node_id,to_node_id,kind) DO NOTHING",
            params![
                edge.root_run_id,
                edge.revision,
                edge.from_node_id,
                edge.to_node_id,
                edge.kind.as_str(),
                payload_text,
                edge.created_by_run_id
            ],
        )
        .map_err(|error| format!("无法写入 evidence 边：{error}"))?;
    if inserted == 1 {
        return Ok(EvidenceInsert::Inserted(
            connection.last_insert_rowid().to_string(),
        ));
    }
    let stored: Option<(i64, String, String)> = query_one(
        connection,
        &format!(
            "SELECT id,payload_json,created_by_run_id FROM agent_evidence_edges WHERE {EDGE_KEY}"
        ),
        &[
            &edge.root_run_id,
            &edge.revision,
            &edge.from_node_id,
            &edge.to_node_id,
            &edge.kind.as_str(),
        ],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        "重放的 evidence 边",
    )?;
    let (found_id, found_payload, found_creator) = stored.ok_or_else(|| {
        format!(
            "evidence 边未写入且找不到同键的记录：{}/{}/{}/{}/{}",
            edge.root_run_id,
            edge.revision,
            edge.from_node_id,
            edge.to_node_id,
            edge.kind.as_str()
        )
    })?;
    let found: JsonValue = decode_json(
        "payload_json",
        "agent_evidence_edges",
        &found_id.to_string(),
        &found_payload,
    )?;
    if found == payload && found_creator == edge.created_by_run_id {
        return Ok(EvidenceInsert::Existing(found_id.to_string()));
    }
    Err(format!(
        "evidence_edge_conflict：同一键已存在不同内容，拒绝覆盖：{} -> {}",
        edge.from_node_id, edge.to_node_id
    ))
}

pub fn list_evidence_edges_at_revision(
    connection: &Connection,
    root_run_id: &str,
    revision: i64,
) -> Result<Vec<EvidenceEdge>, String> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT {EDGE_COLUMNS} FROM agent_evidence_edges WHERE root_run_id=?1 AND revision=?2 ORDER BY id"
        ))
        .map_err(|error| format!("无法准备 evidence 边查询：{error}"))?;
    let rows = statement
        .query_map(params![root_run_id, revision], edge_from_row)
        .map_err(|error| format!("无法读取 evidence 边列表：{error}"))?;
    rows.map(|item| {
        item.map_err(|error| format!("无法解析 evidence 边列表：{error}"))
            .and_then(decode_edge)
    })
    .collect()
}
