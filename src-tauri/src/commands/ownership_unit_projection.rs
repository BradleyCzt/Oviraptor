// IR-02 / IR-11（用户 2026-10-09 批准：“在归属模块加按资产的只读单位投影，暴露面只读使用”）。
//
// 按资产的只读单位投影：每个资产给出 ownerEntityId、关系、证据和版本。纯读取，不写任何表，
// 不加列；确认归属、授权、准入、分组、恢复的语义一律不变。只计入有权威依据的归属：
// - 单位项目：资产为人工确认归属（confirmed/manual），单位来自项目绑定，且该单位仍在已确认目录中。
// - 集团总账：资产当前为人工确认归属，且最近一次改变它的组操作是已完成的“确认归属”组决定；
//   该组的候选实体来自已绑定目录（candidate_source='directory'，lane='consistent'，只有这一类组允许
//   确认归属），实体仍在已确认目录中、属于绑定集团、并且是一个单位（不是集团本身、供应商或未知）；
//   组决定之后没有其它改变归属状态的人工动作。
// 候选组、自动判断、标签/网络线索、单独确认（没有单位）一律不算已归属，也不按名称或区划码推断。
// 撤销（组回退、改判、删除结论）、目录退役、绑定变化都会改变投影和版本。

pub const OWNERSHIP_UNIT_PROJECTION_VERSION: &str = "oviraptor.ownership.unit_projection.v1";

/// 可以作为“资产所属单位”的实体类型。集团本身、供应商、未知类型都不是单位。
const OWNERSHIP_UNIT_KINDS: [&str; 6] = [
    "headquarters",
    "province_company",
    "city_branch",
    "county_branch",
    "professional_subsidiary",
    "overseas_entity",
];

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipAssetUnitEdge {
    pub asset_id: i64,
    pub owner_entity_id: i64,
    /// `project_binding`（单位项目绑定）或 `group_decision`（人工确认的复核组决定）。
    pub relation: String,
    pub evidence: serde_json::Value,
    /// 这条边依据的目录版本、绑定版本和操作，任何一项变化都会改变它。
    pub version: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipUnitProjection {
    pub projection_version: &'static str,
    pub project_id: i64,
    /// `unbound` / `group_ledger` / `unit` / `binding_broken`。
    pub binding_kind: String,
    pub directory_version: i64,
    pub binding_version: i64,
    /// 整个投影的内容版本（目录、绑定、每条边、未分配原因的摘要）。
    pub version: String,
    pub edges: Vec<OwnershipAssetUnitEdge>,
    /// 人工确认归属的现存成员数。
    pub confirmed: i64,
    /// 已确认但没有权威单位依据的资产数，按原因分列。
    pub unassigned: i64,
    pub unassigned_reasons: std::collections::BTreeMap<String, i64>,
}

struct OwnershipProjectionOp {
    project_id: i64,
    kind: String,
    action: String,
    status: String,
    group_id: i64,
    run_id: i64,
}

struct OwnershipProjectionGroup {
    project_id: i64,
    lane: String,
    candidate_entity_id: Option<i64>,
    candidate_source: String,
    relation_kind: String,
    support_kind: String,
}

fn ownership_projection_error(error: impl std::fmt::Display) -> String {
    format!("OWNERSHIP_UNIT_PROJECTION_FAILED: 单位投影读取失败：{error}")
}

/// 整个项目的投影。
pub fn ownership_asset_unit_projection(
    connection: &rusqlite::Connection,
    project_id: i64,
) -> Result<OwnershipUnitProjection, String> {
    ownership_unit_projection_for(connection, project_id, None)
}

/// 单个资产的边（事件自动关联用）。没有权威依据时返回 None。
pub fn ownership_asset_unit_edge(
    connection: &rusqlite::Connection,
    project_id: i64,
    asset_id: i64,
) -> Result<Option<OwnershipAssetUnitEdge>, String> {
    Ok(ownership_unit_projection_for(connection, project_id, Some(&[asset_id]))?
        .edges
        .into_iter()
        .find(|edge| edge.asset_id == asset_id))
}

/// FIX3-P2-1：一批资产的边。目录、实体、项目绑定只读一次，再逐个取这批资产的确认行；
/// 读一个大事件时不再按位置重复加载整份目录。结果与逐个调用 `ownership_asset_unit_edge` 相同。
pub fn ownership_asset_unit_edges(
    connection: &rusqlite::Connection,
    project_id: i64,
    asset_ids: &[i64],
) -> Result<HashMap<i64, OwnershipAssetUnitEdge>, String> {
    if asset_ids.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(ownership_unit_projection_for(connection, project_id, Some(asset_ids))?
        .edges
        .into_iter()
        .map(|edge| (edge.asset_id, edge))
        .collect())
}

fn ownership_unit_projection_for(
    connection: &rusqlite::Connection,
    project_id: i64,
    only_assets: Option<&[i64]>,
) -> Result<OwnershipUnitProjection, String> {
    let directory_version = org_directory_version(connection)?;
    let entities = load_org_entities(connection)?;
    let binding = load_project_org_binding(connection, project_id)?;
    let by_id: HashMap<i64, &OrgEntityView> = entities.iter().map(|entity| (entity.id, entity)).collect();

    // 人工确认归属的现存成员（单独确认、组确认、授权后的行都在这里；自动判断不会是 confirmed）。
    let confirmed_assets: Vec<i64> = if let Some(list) = only_assets {
        // FIX2：指定资产时直接按 (project_id, asset_id) 取，不让 `?2 IS NULL OR` 拖成全项目扫描。
        let mut statement = connection
            .prepare(
                "SELECT pa.asset_id FROM project_assets pa JOIN assets a ON a.id=pa.asset_id \
                 JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=pa.asset_id \
                 WHERE pa.project_id=?1 AND pa.asset_id=?2 AND pa.is_deleted=0 AND o.status='confirmed' AND o.source='manual'",
            )
            .map_err(ownership_projection_error)?;
        let mut wanted: Vec<i64> = list.to_vec();
        wanted.sort_unstable();
        wanted.dedup();
        let mut rows = Vec::new();
        for asset in wanted {
            if let Some(id) = statement
                .query_row(params![project_id, asset], |row| row.get::<_, i64>(0))
                .optional()
                .map_err(ownership_projection_error)?
            {
                rows.push(id);
            }
        }
        rows
    } else {
        let mut statement = connection
            .prepare(
                "SELECT pa.asset_id FROM project_assets pa JOIN assets a ON a.id=pa.asset_id \
                 JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=pa.asset_id \
                 WHERE pa.project_id=?1 AND pa.is_deleted=0 AND o.status='confirmed' AND o.source='manual' \
                 ORDER BY pa.asset_id",
            )
            .map_err(ownership_projection_error)?;
        let rows = statement
            .query_map(params![project_id], |row| row.get::<_, i64>(0))
            .map_err(ownership_projection_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(ownership_projection_error)?;
        rows
    };

    let mut edges = Vec::new();
    let mut reasons: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    let mut unassigned = |reason: &str| *reasons.entry(reason.to_string()).or_insert(0) += 1;
    let mut binding_version = 0;
    let binding_kind = match binding.as_ref() {
        None => {
            for _ in &confirmed_assets {
                unassigned("project_unbound");
            }
            "unbound".to_string()
        }
        Some(row) => {
            binding_version = row.binding_version;
            let group_ok = by_id.contains_key(&row.group_entity_id);
            let unit_entity = row.unit_entity_id.and_then(|id| by_id.get(&id).copied());
            match row.project_kind.as_str() {
                "unit" if group_ok && unit_entity.is_some() => {
                    let unit = unit_entity.expect("checked");
                    for asset_id in &confirmed_assets {
                        edges.push(OwnershipAssetUnitEdge {
                            asset_id: *asset_id,
                            owner_entity_id: unit.id,
                            relation: "project_binding".into(),
                            evidence: serde_json::json!({
                                "basis": "单位项目绑定 + 人工确认归属",
                                "bindingVersion": row.binding_version,
                                "unitEntityId": unit.id,
                            }),
                            version: format!("d{directory_version}.b{}.binding", row.binding_version),
                        });
                    }
                    "unit".to_string()
                }
                "group_ledger" if group_ok => {
                    ownership_group_ledger_edges(
                        connection,
                        project_id,
                        row,
                        &by_id,
                        directory_version,
                        &confirmed_assets,
                        &mut edges,
                        &mut unassigned,
                    )?;
                    "group_ledger".to_string()
                }
                _ => {
                    for _ in &confirmed_assets {
                        unassigned("binding_broken");
                    }
                    "binding_broken".to_string()
                }
            }
        }
    };
    let unassigned_total: i64 = reasons.values().sum();
    let digest_source = serde_json::json!({
        "projection": OWNERSHIP_UNIT_PROJECTION_VERSION,
        "project": project_id,
        "kind": binding_kind,
        "directory": directory_version,
        "binding": binding_version,
        "edges": edges.iter().map(|edge| (edge.asset_id, edge.owner_entity_id, &edge.version)).collect::<Vec<_>>(),
        "unassigned": reasons,
    });
    Ok(OwnershipUnitProjection {
        projection_version: OWNERSHIP_UNIT_PROJECTION_VERSION,
        project_id,
        binding_kind,
        directory_version,
        binding_version,
        version: ownership_sha(&digest_source.to_string())[..16].to_string(),
        confirmed: confirmed_assets.len() as i64,
        unassigned: unassigned_total,
        unassigned_reasons: reasons,
        edges,
    })
}

#[allow(clippy::too_many_arguments)]
fn ownership_group_ledger_edges(
    connection: &rusqlite::Connection,
    project_id: i64,
    binding: &ProjectOrgBindingView,
    by_id: &HashMap<i64, &OrgEntityView>,
    directory_version: i64,
    confirmed_assets: &[i64],
    edges: &mut Vec<OwnershipAssetUnitEdge>,
    unassigned: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let mut last_item = connection
        .prepare(
            "SELECT i.operation_id,i.created_at FROM ownership_operation_items i \
             JOIN ownership_operations op ON op.operation_id=i.operation_id \
             WHERE op.project_id=?1 AND i.asset_id=?2 AND i.outcome='applied' \
             AND op.kind IN ('group_decision','group_revert') ORDER BY i.rowid DESC LIMIT 1",
        )
        .map_err(ownership_projection_error)?;
    let mut decided_event = connection
        .prepare(
            "SELECT MAX(id) FROM asset_events WHERE project_id=?1 AND asset_id=?2 AND event_type='ownership' \
             AND summary LIKE 'confirmed:%' AND instr(summary,'操作 '||?3||'）')>0",
        )
        .map_err(ownership_projection_error)?;
    let mut changed_event = connection
        .prepare(
            "SELECT MAX(id) FROM asset_events WHERE project_id=?1 AND asset_id=?2 AND event_type='ownership' \
             AND summary NOT LIKE 'confirmed%'",
        )
        .map_err(ownership_projection_error)?;
    let mut ops: HashMap<String, Option<OwnershipProjectionOp>> = HashMap::new();
    let mut groups: HashMap<i64, Option<OwnershipProjectionGroup>> = HashMap::new();
    // 规模（P1-REM）：asset_events 只有 (project_id, created_at) 索引，逐资产查事件会对每个资产扫一遍
    // 全项目事件（O(n²)）。整项目投影时一次读出本项目的归属事件，按与 SQL 完全相同的规则在内存里判定。
    let events = if confirmed_assets.len() > 1 { Some(ownership_projection_events(connection, project_id)?) } else { None };
    for asset_id in confirmed_assets {
        let last: Option<(String, String)> = last_item
            .query_row(params![project_id, asset_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()
            .map_err(ownership_projection_error)?;
        let Some((operation_id, decided_at)) = last else {
            // 单独确认（或旧数据）只说“属于当前主体”，没有指明单位。
            unassigned("no_unit_decision");
            continue;
        };
        if !ops.contains_key(&operation_id) {
            let op = connection
                .query_row(
                    "SELECT project_id,kind,action,status,group_id,run_id FROM ownership_operations WHERE operation_id=?1",
                    [&operation_id],
                    |row| {
                        Ok(OwnershipProjectionOp {
                            project_id: row.get(0)?,
                            kind: row.get(1)?,
                            action: row.get(2)?,
                            status: row.get(3)?,
                            group_id: row.get(4)?,
                            run_id: row.get(5)?,
                        })
                    },
                )
                .optional()
                .map_err(ownership_projection_error)?;
            ops.insert(operation_id.clone(), op);
        }
        let Some(op) = ops.get(&operation_id).and_then(|op| op.as_ref()) else {
            unassigned("operation_missing");
            continue;
        };
        if op.project_id != project_id || op.kind != "group_decision" || op.action != "confirm_ownership" {
            // 最近一次组操作是回退或其它动作：确认不是来自这次组决定。
            unassigned("no_unit_decision");
            continue;
        }
        if op.status != "completed" {
            unassigned("operation_in_progress");
            continue;
        }
        if let std::collections::hash_map::Entry::Vacant(slot) = groups.entry(op.group_id) {
            let group = connection
                .query_row(
                    "SELECT project_id,lane,candidate_entity_id,candidate_source,relation_kind,support_kind FROM ownership_review_groups WHERE id=?1 AND run_id=?2",
                    params![op.group_id, op.run_id],
                    |row| {
                        Ok(OwnershipProjectionGroup {
                            project_id: row.get(0)?,
                            lane: row.get(1)?,
                            candidate_entity_id: row.get(2)?,
                            candidate_source: row.get(3)?,
                            relation_kind: row.get(4)?,
                            support_kind: row.get(5)?,
                        })
                    },
                )
                .optional()
                .map_err(ownership_projection_error)?;
            slot.insert(group);
        }
        let Some(group) = groups.get(&op.group_id).and_then(|group| group.as_ref()) else {
            unassigned("group_missing");
            continue;
        };
        let Some(entity_id) = group.candidate_entity_id else {
            unassigned("group_without_directory_entity");
            continue;
        };
        if group.project_id != project_id || group.lane != "consistent" || group.candidate_source != "directory" {
            unassigned("group_without_directory_entity");
            continue;
        }
        let Some(entity) = by_id.get(&entity_id) else {
            unassigned("entity_retired");
            continue;
        };
        if entity.id != binding.group_entity_id && entity.group_id != Some(binding.group_entity_id) {
            unassigned("entity_outside_binding");
            continue;
        }
        if !OWNERSHIP_UNIT_KINDS.contains(&entity.entity_kind.as_str()) {
            unassigned("entity_not_a_unit");
            continue;
        }
        let (decided, changed): (Option<i64>, Option<i64>) = match events.as_ref() {
            Some((confirmed, changed)) => {
                let needle = format!("操作 {operation_id}）");
                (
                    confirmed.get(asset_id).and_then(|rows| rows.iter().filter(|(_, summary)| summary.contains(&needle)).map(|(id, _)| *id).max()),
                    changed.get(asset_id).copied(),
                )
            }
            None => (
                decided_event
                    .query_row(params![project_id, asset_id, operation_id], |row| row.get(0))
                    .map_err(ownership_projection_error)?,
                changed_event.query_row(params![project_id, asset_id], |row| row.get(0)).map_err(ownership_projection_error)?,
            ),
        };
        match (decided, changed) {
            (None, _) => {
                unassigned("decision_event_missing");
                continue;
            }
            (Some(decided), Some(changed)) if changed > decided => {
                // 组决定之后人工改过归属状态（即使之后又单独确认），单位依据已不再是那次组决定。
                unassigned("changed_after_group_decision");
                continue;
            }
            _ => {}
        }
        edges.push(OwnershipAssetUnitEdge {
            asset_id: *asset_id,
            owner_entity_id: entity.id,
            relation: "group_decision".into(),
            evidence: serde_json::json!({
                "basis": "人工确认归属的复核组决定，候选单位来自已绑定目录",
                "operationId": operation_id,
                "groupId": op.group_id,
                "runId": op.run_id,
                "support": group.support_kind,
                "relationKind": group.relation_kind,
                "decidedAt": decided_at,
            }),
            version: format!("d{directory_version}.b{}.op:{operation_id}", binding.binding_version),
        });
    }
    Ok(())
}

type OwnershipProjectionEvents = (HashMap<i64, Vec<(i64, String)>>, HashMap<i64, i64>);

/// 一次读出本项目的归属事件：`confirmed:` 开头的按资产保留（id, 摘要），其余取每个资产的最大 id。
/// 前缀比较与 SQLite `LIKE` 一致（ASCII 不分大小写）；摘要为 NULL 的行两边都不算。
fn ownership_projection_events(connection: &rusqlite::Connection, project_id: i64) -> Result<OwnershipProjectionEvents, String> {
    let mut statement = connection
        .prepare("SELECT id,asset_id,summary FROM asset_events WHERE project_id=?1 AND event_type='ownership' AND asset_id IS NOT NULL")
        .map_err(ownership_projection_error)?;
    let mut rows = statement.query([project_id]).map_err(ownership_projection_error)?;
    let mut confirmed: HashMap<i64, Vec<(i64, String)>> = HashMap::new();
    let mut changed: HashMap<i64, i64> = HashMap::new();
    let starts = |summary: &str, prefix: &[u8]| summary.len() >= prefix.len() && summary.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix);
    while let Some(row) = rows.next().map_err(ownership_projection_error)? {
        let id: i64 = row.get(0).map_err(ownership_projection_error)?;
        let asset_id: i64 = row.get(1).map_err(ownership_projection_error)?;
        let Some(summary) = row.get::<_, Option<String>>(2).map_err(ownership_projection_error)? else {
            continue;
        };
        if starts(&summary, b"confirmed:") {
            confirmed.entry(asset_id).or_default().push((id, summary.clone()));
        }
        if !starts(&summary, b"confirmed") {
            let slot = changed.entry(asset_id).or_insert(id);
            *slot = (*slot).max(id);
        }
    }
    Ok((confirmed, changed))
}

#[cfg(test)]
mod ownership_unit_projection_tests {
    use super::ownership_review_o2_tests::{apply_input, count, fixture, group_of, request, run_analysis};
    use super::*;

    fn confirm_group(connection: &mut rusqlite::Connection, run: i64, asset_id: i64, operation_id: &str) -> i64 {
        let group = group_of(connection, run, asset_id);
        let preview = preview_ownership_group_decision_for_connection(connection, request(run, group.id, "confirm_ownership")).unwrap();
        let result = apply_ownership_group_decision_for_connection(connection, apply_input(&preview, operation_id)).unwrap();
        assert_eq!(result.status, "completed");
        group.id
    }

    fn edge_of(projection: &OwnershipUnitProjection, asset_id: i64) -> Option<&OwnershipAssetUnitEdge> {
        projection.edges.iter().find(|edge| edge.asset_id == asset_id)
    }

    fn manual(connection: &mut rusqlite::Connection, asset_id: i64, status: &str) {
        let expected = vec![ownership_expectation_of(connection, 91, asset_id).unwrap()];
        apply_asset_ownership_decisions(
            connection,
            AssetOwnershipDecisionInput {
                selections: vec![AssetSelection { project_id: 91, asset_id }],
                status: status.into(),
                note: "人工".into(),
                learn_domain_rule: false,
            },
            91,
            expected,
        )
        .unwrap();
    }

    #[test]
    fn unit_projection_counts_only_confirmed_directory_group_decisions() {
        let (root, mut connection) = fixture();
        let run = run_analysis(&mut connection, 91, 50);
        // 候选组都还没有人确认：只有 14 号（单独人工确认，没有单位）是确认归属，且不分配单位。
        let before_changes = connection.total_changes();
        let empty = ownership_asset_unit_projection(&connection, 91).unwrap();
        assert_eq!(connection.total_changes(), before_changes, "投影只读");
        assert_eq!(empty.binding_kind, "group_ledger");
        assert_eq!((empty.confirmed, empty.unassigned, empty.edges.len()), (1, 1, 0), "{:?}", empty.unassigned_reasons);
        assert_eq!(empty.unassigned_reasons.get("no_unit_decision"), Some(&1));
        let candidate: Option<i64> = connection
            .query_row("SELECT candidate_entity_id FROM ownership_review_groups WHERE id=?1", [group_of(&connection, run, 1).id], |row| row.get(0))
            .unwrap();
        assert_eq!(candidate, Some(2), "1 号所在组有候选单位，但候选不算已归属");
        // 单独确认一条有候选单位的资产：仍然没有单位（不从候选组推断）。
        manual(&mut connection, 3, "confirmed");
        let single = ownership_asset_unit_projection(&connection, 91).unwrap();
        assert!(edge_of(&single, 3).is_none(), "单独确认不带单位：{:?}", single.edges);
        assert_eq!(single.unassigned_reasons.get("no_unit_decision"), Some(&2));

        // 组决定：浙江省公司组（1、2 号）与专业公司组（6 号）。
        confirm_group(&mut connection, run, 1, "op-proj-zj-0001");
        confirm_group(&mut connection, run, 6, "op-proj-sz-0001");
        let projection = ownership_asset_unit_projection(&connection, 91).unwrap();
        let one = edge_of(&projection, 1).expect("1 号有单位");
        assert_eq!((one.owner_entity_id, one.relation.as_str()), (2, "group_decision"));
        assert_eq!(one.evidence["operationId"], "op-proj-zj-0001");
        assert_eq!(one.version, "d1.b1.op:op-proj-zj-0001");
        assert_eq!(edge_of(&projection, 2).map(|edge| edge.owner_entity_id), Some(2));
        assert_eq!(edge_of(&projection, 6).map(|edge| edge.owner_entity_id), Some(4), "专业公司按目录实体，不按名称或地区归省");
        assert!(edge_of(&projection, 14).is_none());
        assert!(edge_of(&projection, 15).is_none(), "自动判断（attributed）不是已归属");
        assert_ne!(projection.version, single.version);
        assert_eq!(projection.confirmed, count(&connection, "SELECT COUNT(*) FROM asset_ownership_decisions o JOIN project_assets pa ON pa.project_id=o.project_id AND pa.asset_id=o.asset_id WHERE o.project_id=91 AND pa.is_deleted=0 AND o.status='confirmed'"));
        assert_eq!(projection.confirmed, projection.edges.len() as i64 + projection.unassigned);
        // 单个资产查询与整体一致。
        assert_eq!(ownership_asset_unit_edge(&connection, 91, 6).unwrap().as_ref(), edge_of(&projection, 6));
        assert!(ownership_asset_unit_edge(&connection, 91, 14).unwrap().is_none());

        // 跨项目：同一资产在项目 92 被人工确认，不能借用项目 91 的组决定。
        connection
            .execute_batch(
                "INSERT INTO project_assets(project_id,asset_id) VALUES(92,1);
                 INSERT INTO asset_ownership_decisions(project_id,asset_id,status,confidence,authorization_status,exposure_eligible,source,reason) VALUES(92,1,'confirmed',100,'unknown',0,'manual','香港项目人工确认');
                 INSERT INTO project_org_bindings(project_id,project_kind,group_entity_id,binding_version) VALUES(92,'group_ledger',1,1);",
            )
            .unwrap();
        let other = ownership_asset_unit_projection(&connection, 92).unwrap();
        assert!(other.edges.is_empty(), "项目 92 不能串用项目 91 的单位：{:?}", other.edges);
        assert_eq!(other.unassigned_reasons.get("no_unit_decision"), Some(&1));
        assert!(ownership_asset_unit_edge(&connection, 92, 1).unwrap().is_none());

        // 操作未完成：不计入。
        connection.execute("UPDATE ownership_operations SET status='applying' WHERE operation_id='op-proj-sz-0001'", []).unwrap();
        let applying = ownership_asset_unit_projection(&connection, 91).unwrap();
        assert!(edge_of(&applying, 6).is_none());
        assert_eq!(applying.unassigned_reasons.get("operation_in_progress"), Some(&1));
        connection.execute("UPDATE ownership_operations SET status='completed' WHERE operation_id='op-proj-sz-0001'", []).unwrap();
        cleanup(root, connection);
    }

    #[test]
    fn unit_projection_drops_reverted_changed_retired_and_rebound_edges() {
        let (root, mut connection) = fixture();
        let run = run_analysis(&mut connection, 91, 50);
        // 撤销组决定（组回退）：不再计入；回退之后单独确认也不恢复单位。
        confirm_group(&mut connection, run, 3, "op-proj-js-0002");
        let js = ownership_asset_unit_projection(&connection, 91).unwrap();
        assert_eq!(edge_of(&js, 3).map(|edge| edge.owner_entity_id), Some(3));
        let revert_preview = preview_ownership_group_revert_for_connection(
            &connection,
            OwnershipRevertRequest { project_id: 91, target_operation_id: "op-proj-js-0002".into(), note: String::new() },
        )
        .unwrap();
        let (reverted, _) = apply_ownership_group_revert_budgeted(
            &mut connection,
            OwnershipRevertApply {
                operation_id: "op-proj-rv-0002".into(),
                project_id: 91,
                target_operation_id: revert_preview.target_operation_id.clone(),
                note: String::new(),
                expected_revert_hash: revert_preview.expected_revert_hash.clone(),
            },
            None,
            500,
        )
        .unwrap();
        assert_eq!((reverted.status.as_str(), reverted.applied), ("completed", 1));
        let after_revert = ownership_asset_unit_projection(&connection, 91).unwrap();
        assert!(edge_of(&after_revert, 3).is_none(), "撤销后不再计入：{:?}", after_revert.edges);
        assert_ne!(after_revert.version, js.version);
        manual(&mut connection, 3, "confirmed");
        let reconfirmed = ownership_asset_unit_projection(&connection, 91).unwrap();
        assert!(edge_of(&reconfirmed, 3).is_none(), "撤销后单独确认也不恢复单位");
        assert!(reconfirmed.unassigned_reasons.get("no_unit_decision").copied().unwrap_or(0) >= 2);

        confirm_group(&mut connection, run, 1, "op-proj-zj-0002");
        let base = ownership_asset_unit_projection(&connection, 91).unwrap();
        assert!(edge_of(&base, 1).is_some() && edge_of(&base, 2).is_some());

        // 授权（allowed）不改变归属事实（O3 语义不变），边保留。
        manual(&mut connection, 1, "confirmed");
        let authorized = ownership_asset_unit_projection(&connection, 91).unwrap();
        assert_eq!(edge_of(&authorized, 1).map(|edge| edge.owner_entity_id), Some(2), "授权不改变单位归属");

        // 组决定之后人工改判，再单独确认：单位依据已不是那次组决定。
        manual(&mut connection, 2, "excluded");
        manual(&mut connection, 2, "confirmed");
        let changed = ownership_asset_unit_projection(&connection, 91).unwrap();
        assert!(edge_of(&changed, 2).is_none(), "改判后不再计入");
        assert_eq!(changed.unassigned_reasons.get("changed_after_group_decision"), Some(&1));
        assert_ne!(changed.version, authorized.version);

        // 目录只升版本：边不变，但版本变化。
        connection.execute("UPDATE org_directory_state SET version=2 WHERE id=1", []).unwrap();
        let bumped = ownership_asset_unit_projection(&connection, 91).unwrap();
        assert_eq!(edge_of(&bumped, 1).map(|edge| edge.version.as_str()), Some("d2.b1.op:op-proj-zj-0002"));
        assert_ne!(bumped.version, changed.version, "目录版本变化必须反映在投影版本里");

        // 目录退役该单位：不再计入。
        connection.execute("UPDATE org_entities SET status='retired' WHERE id=2", []).unwrap();
        let retired = ownership_asset_unit_projection(&connection, 91).unwrap();
        assert!(edge_of(&retired, 1).is_none());
        // 1 号和已改判的 2 号都指向浙江省公司：单位核查先于事件核查。
        assert_eq!(retired.unassigned_reasons.get("entity_retired"), Some(&2));
        connection.execute("UPDATE org_entities SET status='confirmed' WHERE id=2", []).unwrap();

        // 项目改绑到别的集团：原单位不在绑定范围内。
        connection.execute("UPDATE project_org_bindings SET group_entity_id=6,binding_version=2 WHERE project_id=91", []).unwrap();
        let rebound = ownership_asset_unit_projection(&connection, 91).unwrap();
        assert!(rebound.edges.is_empty());
        assert_eq!(rebound.unassigned_reasons.get("entity_outside_binding"), Some(&2));
        connection.execute("UPDATE project_org_bindings SET group_entity_id=1,binding_version=3 WHERE project_id=91", []).unwrap();

        cleanup(root, connection);
    }

    #[test]
    fn unit_projection_ignores_forged_candidate_groups_and_unit_projects_use_the_binding() {
        let (root, mut connection) = fixture();
        let run = run_analysis(&mut connection, 91, 50);
        // 冲突组（11 号）不允许确认归属；伪造一条“确认”操作也不能把候选单位算成已归属。
        let conflict = group_of(&connection, run, 11);
        let lane: String = connection
            .query_row("SELECT lane FROM ownership_review_groups WHERE id=?1", [conflict.id], |row| row.get(0))
            .unwrap();
        assert_ne!(lane, "consistent");
        connection
            .execute_batch(&format!(
                "INSERT INTO ownership_operations(operation_id,project_id,run_id,group_id,kind,action,status,payload_hash,expected_action_hash,result_json) VALUES('op-forged-0001',91,{run},{gid},'group_decision','confirm_ownership','completed','h','h','{{}}');
                 INSERT INTO ownership_operation_items(operation_id,asset_id,outcome) VALUES('op-forged-0001',11,'applied');
                 INSERT INTO asset_ownership_decisions(project_id,asset_id,status,confidence,authorization_status,exposure_eligible,source,reason) VALUES(91,11,'confirmed',100,'unknown',0,'manual','伪造');
                 INSERT INTO asset_events(project_id,asset_id,event_type,summary) VALUES(91,11,'ownership','confirmed: 伪造（复核组 #{gid}，快照 #{run}，操作 op-forged-0001）');",
                gid = conflict.id
            ))
            .unwrap();
        let projection = ownership_asset_unit_projection(&connection, 91).unwrap();
        assert!(edge_of(&projection, 11).is_none(), "候选/冲突组不算已归属：{:?}", projection.edges);
        assert_eq!(projection.unassigned_reasons.get("group_without_directory_entity"), Some(&1));

        // 即使组被改成“一致+目录”，指向的若不是单位（集团本身）也不算；决定事件缺失也不算。
        connection
            .execute("UPDATE ownership_review_groups SET lane='consistent',candidate_source='directory',candidate_entity_id=1 WHERE id=?1", [conflict.id])
            .unwrap();
        let not_unit = ownership_asset_unit_projection(&connection, 91).unwrap();
        assert!(edge_of(&not_unit, 11).is_none(), "集团本身不是单位：{:?}", not_unit.edges);
        assert_eq!(not_unit.unassigned_reasons.get("entity_not_a_unit"), Some(&1));
        connection
            .execute_batch(&format!(
                "UPDATE ownership_review_groups SET candidate_entity_id=2 WHERE id={gid};
                 DELETE FROM asset_events WHERE project_id=91 AND asset_id=11 AND instr(summary,'op-forged-0001')>0;",
                gid = conflict.id
            ))
            .unwrap();
        let no_event = ownership_asset_unit_projection(&connection, 91).unwrap();
        assert!(edge_of(&no_event, 11).is_none(), "没有决定事件不算：{:?}", no_event.edges);
        assert_eq!(no_event.unassigned_reasons.get("decision_event_missing"), Some(&1));

        // 单位项目：人工确认的资产归绑定单位；自动判断不算。
        connection
            .execute_batch(
                "UPDATE project_org_bindings SET project_kind='unit',unit_entity_id=4,binding_version=7 WHERE project_id=91;",
            )
            .unwrap();
        let unit = ownership_asset_unit_projection(&connection, 91).unwrap();
        assert_eq!(unit.binding_kind, "unit");
        assert!(unit.edges.iter().all(|edge| edge.owner_entity_id == 4 && edge.relation == "project_binding"));
        assert_eq!(unit.edges.len() as i64, unit.confirmed);
        assert!(edge_of(&unit, 15).is_none(), "attributed 不计入");
        // 绑定的单位退役：按失效绑定处理。
        connection.execute("UPDATE org_entities SET status='retired' WHERE id=4", []).unwrap();
        let broken = ownership_asset_unit_projection(&connection, 91).unwrap();
        assert_eq!((broken.binding_kind.as_str(), broken.edges.len()), ("binding_broken", 0));
        // 未绑定项目：全部未分配。
        let unbound = ownership_asset_unit_projection(&connection, 92).unwrap();
        assert_eq!(unbound.binding_kind, "unbound");
        let _ = &mut connection;
        cleanup(root, connection);
    }

    #[test]
    fn exposure_reads_the_projection_per_unit_and_never_puts_a_subsidiary_under_a_province() {
        let (root, mut connection) = fixture();
        crate::commands::exposure::state::ensure_exposure_schema(&connection).unwrap();
        let run = run_analysis(&mut connection, 91, 50);
        confirm_group(&mut connection, run, 1, "op-proj-zj-0003");
        confirm_group(&mut connection, run, 6, "op-proj-sz-0003");
        let before = connection.total_changes();
        let units = crate::commands::exposure::units::project_units(&connection, 91).unwrap();
        assert_eq!(connection.total_changes(), before, "暴露面只读");
        assert_eq!(units.per_asset_edge, "ownership_projection");
        assert_eq!(units.counts.confirmed_unit, Some(3), "集团总账有了真实单位数");
        assert_eq!(units.counts.unassigned, 1, "14 号没有权威单位依据");
        let zj = units.per_unit.iter().find(|row| row.unit.entity_id == 2).expect("浙江");
        assert_eq!((zj.bucket.as_str(), zj.assets), ("province", 2));
        let sz = units.per_unit.iter().find(|row| row.unit.entity_id == 4).expect("专业公司");
        assert_eq!((sz.bucket.as_str(), sz.assets), ("professional_subsidiary", 1));
        assert!(sz.province.is_none(), "专业公司不归省");
        assert_eq!(units.ownership_version.len(), 16);
        cleanup(root, connection);
    }

    fn cleanup(root: PathBuf, connection: rusqlite::Connection) {
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }
}
