// G1-2 — the asset→unit projection of one project, read-only. It answers "how many of this
// project's assets can be attributed to which unit" from the authoritative organisation
// directory and the project binding only: never from a name, a region code or a guess.
//
// The directory is reused read-only through the shared ownership functions
// (`load_org_entities`, `load_project_org_binding`, `org_binding_scope`,
// `org_directory_version`). IR-02/IR-11 (approved 2026-10-09): the per-asset unit edge comes
// from the ownership module's read-only projection (`ownership_asset_unit_projection`). A
// group-ledger asset has a unit only when a person confirmed a directory-backed review group;
// everything else stays `unassigned`, never spread over provinces by name or region code.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

pub const UNIT_PROJECTION_VERSION: &str = "oviraptor.exposure.units.v1";

/// Where the bound unit sits in the directory, from its kind and parent chain only.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UnitRef {
    pub entity_id: i64,
    pub legal_name: String,
    pub entity_kind: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UnitCounts {
    /// Project members (not deleted).
    pub members: i64,
    /// Members with an automatic or "related" candidate link, not a confirmation.
    pub candidate_linked: i64,
    /// Confirmed as the group's asset; `None` when the project is not bound to a group.
    pub confirmed_group: Option<i64>,
    /// Confirmed as the bound unit's asset; only a `unit` project can say this.
    pub confirmed_unit: Option<i64>,
    /// Confirmed assets whose unit is not known (unbound project, or group ledger without a
    /// per-asset unit edge).
    pub unassigned: i64,
    /// Confirmed but not (or no longer) authorized for exposure collection.
    pub pending_authorization: i64,
    /// Admission-passing members (the collector's own predicate).
    pub eligible: i64,
    /// Passive-web work items planned / completed in the latest round, by distinct asset.
    pub planned: i64,
    pub executed: i64,
    /// Eligible members the latest round did not complete (never ran, blocked, failed).
    pub uncovered: i64,
    pub latest_run_id: Option<i64>,
}

/// Assets per unit from the ownership projection, placed by the directory parent chain.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitAssetCount {
    pub unit: UnitRef,
    pub bucket: String,
    pub province: Option<UnitRef>,
    pub assets: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitProjection {
    pub projection_version: &'static str,
    /// `unbound` / `group_ledger` / `unit` / `binding_broken`.
    pub binding_status: String,
    pub group: Option<UnitRef>,
    pub unit: Option<UnitRef>,
    /// `province` / `city` / `county` / `professional_subsidiary` / `headquarters` / `group`
    /// / `overseas` / `supplier` / `unknown` / `none`.
    pub unit_bucket: String,
    /// The province company above the bound unit, reached through city/county parents only.
    /// A professional subsidiary (or anything under one) never gets a province here.
    pub unit_province: Option<UnitRef>,
    pub directory_version: i64,
    pub binding_version: i64,
    pub scope_entities: i64,
    pub counts: UnitCounts,
    /// `ownership_projection`: per-asset units come from the ownership module (IR-02/IR-11).
    pub per_asset_edge: &'static str,
    /// Content version of the ownership projection this answer was built from.
    pub ownership_version: String,
    pub per_unit: Vec<UnitAssetCount>,
    /// Why confirmed assets have no unit (`no_unit_decision`, `entity_retired`, ...).
    pub unassigned_reasons: std::collections::BTreeMap<String, i64>,
    /// asset id → owner entity id, for read-only consumers in this crate.
    #[serde(skip)]
    pub asset_units: HashMap<i64, i64>,
    pub notes: Vec<String>,
}

fn unit_ref(entity: &crate::commands::ownership::OrgEntityView) -> UnitRef {
    UnitRef {
        entity_id: entity.id,
        legal_name: entity.legal_name.clone(),
        entity_kind: entity.entity_kind.clone(),
    }
}

/// The bucket and province of a unit from its kind and parent chain. Only city/county
/// branches are walked upwards, and only a `province_company` ends the walk with a province.
/// A professional subsidiary, headquarters, group, overseas entity, supplier, unknown kind,
/// missing parent or cycle ends it without one.
pub fn place_unit(
    unit: &crate::commands::ownership::OrgEntityView,
    by_id: &HashMap<i64, &crate::commands::ownership::OrgEntityView>,
) -> (String, Option<UnitRef>, Option<String>) {
    let bucket = match unit.entity_kind.as_str() {
        "province_company" => "province",
        "city_branch" => "city",
        "county_branch" => "county",
        "professional_subsidiary" => "professional_subsidiary",
        "headquarters" => "headquarters",
        "group" => "group",
        "overseas_entity" => "overseas",
        "supplier" => "supplier",
        _ => "unknown",
    };
    if bucket == "province" {
        return (bucket.to_string(), Some(unit_ref(unit)), None);
    }
    if bucket != "city" && bucket != "county" {
        return (bucket.to_string(), None, None);
    }
    let mut seen: HashSet<i64> = HashSet::from([unit.id]);
    let mut cursor = unit.parent_id;
    while let Some(parent_id) = cursor {
        if !seen.insert(parent_id) {
            return (bucket.to_string(), None, Some(format!("目录父级链在实体 {parent_id} 处成环，未判定所属省公司")));
        }
        let Some(parent) = by_id.get(&parent_id) else {
            return (bucket.to_string(), None, Some(format!("父级实体 {parent_id} 不在已确认目录中，未判定所属省公司")));
        };
        match parent.entity_kind.as_str() {
            "province_company" => return (bucket.to_string(), Some(unit_ref(parent)), None),
            "city_branch" | "county_branch" => cursor = parent.parent_id,
            "professional_subsidiary" => {
                return (
                    "professional_subsidiary".to_string(),
                    None,
                    Some(format!("上级是专业公司「{}」，不归入任何省公司", parent.legal_name)),
                )
            }
            other => {
                return (bucket.to_string(), None, Some(format!("上级是 {other}，没有省公司")))
            }
        }
    }
    (bucket.to_string(), None, Some("没有上级实体，未判定所属省公司".to_string()))
}

fn storage(error: impl std::fmt::Display) -> String {
    format!("[exposure_unit_projection_failed] 单位投影读取失败：{error}")
}

/// Builds the projection. Read-only: no row is written, the shared directory is not copied.
pub fn project_units(connection: &Connection, project_id: i64) -> Result<UnitProjection, String> {
    let directory_version = crate::commands::ownership::org_directory_version(connection).map_err(storage)?;
    let entities = crate::commands::ownership::load_org_entities(connection).map_err(storage)?;
    let binding = crate::commands::ownership::load_project_org_binding(connection, project_id).map_err(storage)?;
    let by_id: HashMap<i64, &crate::commands::ownership::OrgEntityView> =
        entities.iter().map(|entity| (entity.id, entity)).collect();
    let scope = crate::commands::ownership::org_binding_scope(&entities, binding.as_ref());
    let mut notes = Vec::new();

    let (members, candidate_linked, confirmed, pending_authorization, eligible): (i64, i64, i64, i64, i64) = connection
        .query_row(
            "SELECT COUNT(*),\
               COALESCE(SUM(d.status IN ('attributed','related')),0),\
               COALESCE(SUM(d.status='confirmed'),0),\
               COALESCE(SUM(d.status='confirmed' AND COALESCE(d.exposure_eligible,0)=0),0),\
               COALESCE(SUM(COALESCE(d.exposure_eligible,0)=1),0) \
             FROM project_assets pa JOIN assets a ON a.id=pa.asset_id \
             LEFT JOIN asset_ownership_decisions d ON d.project_id=pa.project_id AND d.asset_id=pa.asset_id \
             WHERE pa.project_id=?1 AND pa.is_deleted=0",
            [project_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .map_err(storage)?;
    let latest_run_id: Option<i64> = connection
        .query_row("SELECT MAX(id) FROM exposure_runs WHERE project_id=?1", [project_id], |row| row.get(0))
        .optional()
        .map_err(storage)?
        .flatten();
    let (planned, executed, executed_eligible): (i64, i64, i64) = match latest_run_id {
        None => (0, 0, 0),
        Some(run_id) => connection
            .query_row(
                "SELECT COUNT(DISTINCT w.asset_id),\
                   COUNT(DISTINCT CASE WHEN w.status='completed' THEN w.asset_id END),\
                   COUNT(DISTINCT CASE WHEN w.status='completed' AND COALESCE(d.exposure_eligible,0)=1 AND pa.is_deleted=0 THEN w.asset_id END) \
                 FROM exposure_work_items w \
                 LEFT JOIN asset_ownership_decisions d ON d.project_id=w.project_id AND d.asset_id=w.asset_id \
                 LEFT JOIN project_assets pa ON pa.project_id=w.project_id AND pa.asset_id=w.asset_id \
                 WHERE w.run_id=?1 AND w.project_id=?2 AND w.kind='passive_web' AND w.asset_id IS NOT NULL",
                params![run_id, project_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(storage)?,
    };

    let mut group = None;
    let mut unit = None;
    let mut unit_bucket = "none".to_string();
    let mut unit_province = None;
    let mut binding_version = 0;
    let binding_status = match binding.as_ref() {
        None => {
            notes.push("项目未绑定组织目录：确认资产全部记为“未分配单位”，不按名称或地区推断".to_string());
            "unbound".to_string()
        }
        Some(row) => {
            binding_version = row.binding_version;
            let group_entity = by_id.get(&row.group_entity_id).copied();
            let unit_entity = row.unit_entity_id.and_then(|id| by_id.get(&id).copied());
            let unit_missing = row.project_kind == "unit" && unit_entity.is_none();
            if group_entity.is_none() || unit_missing || !matches!(row.project_kind.as_str(), "group_ledger" | "unit") {
                notes.push(format!(
                    "项目绑定（v{}）指向的实体已不在已确认目录中或类型无效，按未绑定处理",
                    row.binding_version
                ));
                "binding_broken".to_string()
            } else {
                group = group_entity.map(unit_ref);
                if let Some(entity) = unit_entity {
                    unit = Some(unit_ref(entity));
                    let (bucket, province, note) = place_unit(entity, &by_id);
                    unit_bucket = bucket;
                    unit_province = province;
                    notes.extend(note);
                } else {
                    unit_bucket = "group".to_string();
                }
                row.project_kind.clone()
            }
        }
    };
    let ownership = crate::commands::ownership::ownership_asset_unit_projection(connection, project_id).map_err(storage)?;
    let asset_units: HashMap<i64, i64> = ownership.edges.iter().map(|edge| (edge.asset_id, edge.owner_entity_id)).collect();
    let mut per_unit_counts: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
    for entity_id in asset_units.values() {
        *per_unit_counts.entry(*entity_id).or_insert(0) += 1;
    }
    let per_unit: Vec<UnitAssetCount> = per_unit_counts
        .into_iter()
        .filter_map(|(entity_id, assets)| {
            let entity = by_id.get(&entity_id)?;
            let (bucket, province, _) = place_unit(entity, &by_id);
            Some(UnitAssetCount { unit: unit_ref(entity), bucket, province, assets })
        })
        .collect();
    let (confirmed_group, confirmed_unit, unassigned) = match binding_status.as_str() {
        "unit" => (Some(confirmed), Some(confirmed), 0),
        "group_ledger" => {
            let placed = asset_units.len() as i64;
            notes.push(format!(
                "集团总账：按归属模块逐资产投影，{placed} 个确认资产有单位，{} 个没有权威单位依据，记为“未分配单位”",
                confirmed.saturating_sub(placed)
            ));
            (Some(confirmed), Some(placed), confirmed.saturating_sub(placed))
        }
        _ => (None, None, confirmed),
    };
    if latest_run_id.is_none() {
        notes.push("本项目还没有采集轮次：计划与已执行均为 0".to_string());
    }
    Ok(UnitProjection {
        projection_version: UNIT_PROJECTION_VERSION,
        binding_status,
        group,
        unit,
        unit_bucket,
        unit_province,
        directory_version,
        binding_version,
        scope_entities: scope.len() as i64,
        counts: UnitCounts {
            members,
            candidate_linked,
            confirmed_group,
            confirmed_unit,
            unassigned,
            pending_authorization,
            eligible,
            planned,
            executed,
            uncovered: eligible.saturating_sub(executed_eligible),
            latest_run_id,
        },
        per_asset_edge: "ownership_projection",
        ownership_version: ownership.version,
        per_unit,
        unassigned_reasons: ownership.unassigned_reasons,
        asset_units,
        notes,
    })
}
