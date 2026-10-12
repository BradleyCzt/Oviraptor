// §Q6 — persistence for the cluster graph. Members keep their own rows: clustering never
// deletes an asset, and an inherited row always names the asset it came from.

use super::clusters::{Cluster, ClusterKind};
use super::contract::ExposureQueryError;
use super::transaction::ImmediateTx;
use rusqlite::{params, Connection, OptionalExtension};

pub const MEMBERSHIP_REPRESENTATIVE: &str = "representative";
pub const MEMBERSHIP_MEMBER: &str = "member";
pub const EVIDENCE_DIRECT: &str = "direct";
pub const EVIDENCE_INHERITED: &str = "inherited";
pub const EVIDENCE_STALE: &str = "stale_representative_revoked";

#[derive(Clone, Debug)]
pub struct PersistedMember {
    pub asset_id: i64,
    pub membership: String,
    pub evidence_source: String,
    pub inherited_from_asset_id: Option<i64>,
    pub verified_at: String,
}

#[derive(Clone, Debug)]
pub struct PersistedCluster {
    pub id: i64,
    pub project_id: i64,
    pub cluster_key: String,
    pub kind: String,
    pub representative_asset_id: Option<i64>,
    pub rule_version: String,
    pub members: Vec<PersistedMember>,
}

/// Replaces one cluster's graph atomically. Nothing outside this cluster is touched, so a
/// re-grouping cannot delete assets or another project's rows.
pub fn save(
    connection: &Connection,
    project_id: i64,
    cluster: &Cluster,
    verified_at: &str,
    run_id: i64,
) -> Result<i64, String> {
    // A refinement graph has no per-member observation: only the representative counts as
    // directly evidenced, everything else is explicitly inherited from it.
    let members: Vec<ObservedMember> = cluster
        .members
        .iter()
        .map(|asset_id| ObservedMember {
            asset_id: *asset_id,
            origin: String::new(),
            content_hash: String::new(),
            confidence: 0,
            observation_id: 0,
            observed_at: if *asset_id == cluster.representative {
                verified_at.to_string()
            } else {
                String::new()
            },
            carries_finding: *asset_id == cluster.representative,
            admitted: true,
        })
        .collect();
    save_observed(connection, project_id, cluster, run_id, 0, &members, None)
}

/// One observation-backed cluster member: the row that put an asset into the graph, and
/// whether that asset carries its own conclusion or merely inherits the representative's.
#[derive(Clone, Debug)]
pub struct ObservedMember {
    pub asset_id: i64,
    pub origin: String,
    pub content_hash: String,
    pub confidence: i64,
    pub observation_id: i64,
    pub observed_at: String,
    pub carries_finding: bool,
    pub admitted: bool,
}

impl ObservedMember {
    fn evidenced(&self) -> bool {
        self.admitted && self.carries_finding
    }
}

/// Writes a cluster with the provenance of every member. Idempotent: the member rows are
/// replaced inside the same transaction, so a retry after a crash converges on one graph
/// instead of accumulating rows for the same cluster key.
pub fn save_observed(
    connection: &Connection,
    project_id: i64,
    cluster: &Cluster,
    run_id: i64,
    settings_version: i64,
    members: &[ObservedMember],
    representative_observation_id: Option<i64>,
) -> Result<i64, String> {
    let direct = members.iter().filter(|row| row.evidenced()).count() as i64;
    let inherited = members
        .iter()
        .filter(|row| row.admitted && !row.carries_finding)
        .count() as i64;
    let dropped = members.iter().filter(|row| !row.admitted).count() as i64;
    let gap_note = if dropped > 0 {
        format!("{dropped} 个成员已不再满足准入，簇结论对其不再成立，必须重新直接观察")
    } else if inherited > 0 && direct == 0 {
        "簇内没有成员自带结论，全部只是分组线索".to_string()
    } else {
        String::new()
    };
    let (boundary_basis, psl_version) = super::clusters::boundary_of_key(&cluster.cluster_key);
    let transaction = ImmediateTx::begin(connection)?;
    transaction.connection().execute(
        "INSERT INTO exposure_clusters(project_id,cluster_key,kind,representative_asset_id,rule_version,member_count,direct_count,inherited_count,representative_observation_id,representative_run_id,settings_version,gap_note,boundary_basis,psl_version,updated_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,datetime('now','localtime'))
         ON CONFLICT(project_id,cluster_key) DO UPDATE SET
            boundary_basis=excluded.boundary_basis,
            psl_version=excluded.psl_version,
            representative_asset_id=excluded.representative_asset_id,
            rule_version=excluded.rule_version,
            member_count=excluded.member_count,
            direct_count=excluded.direct_count,
            inherited_count=excluded.inherited_count,
            representative_observation_id=excluded.representative_observation_id,
            representative_run_id=excluded.representative_run_id,
            settings_version=excluded.settings_version,
            gap_note=excluded.gap_note,
            updated_at=datetime('now','localtime')",
        params![
            project_id,
            cluster.cluster_key,
            cluster.kind.as_str(),
            cluster.representative,
            cluster.rule_version,
            cluster.members.len() as i64,
            direct,
            inherited,
            representative_observation_id.filter(|value: &i64| *value > 0),
            run_id,
            settings_version,
            gap_note,
            boundary_basis,
            psl_version
        ],
    )
    .map_err(|error| format!("簇写入失败：{error}"))?;
    let id: i64 = transaction
        .connection()
        .query_row(
            "SELECT id FROM exposure_clusters WHERE project_id=?1 AND cluster_key=?2",
            params![project_id, cluster.cluster_key],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    transaction
        .connection()
        .execute(
            "DELETE FROM exposure_cluster_members WHERE cluster_id=?1",
            [id],
        )
        .map_err(|error| error.to_string())?;
    for asset_id in &cluster.members {
        let representative = *asset_id == cluster.representative;
        let detail = members.iter().find(|row| row.asset_id == *asset_id);
        let evidence = match detail {
            None => EVIDENCE_INHERITED,
            Some(row) if !row.admitted => EVIDENCE_STALE,
            Some(row) if row.carries_finding => EVIDENCE_DIRECT,
            Some(_) => EVIDENCE_INHERITED,
        };
        transaction.connection().execute(
            "INSERT INTO exposure_cluster_members(cluster_id,asset_id,project_id,membership,evidence_source,inherited_from_asset_id,inherited_run_id,observation_id,rule_version,verified_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                id,
                asset_id,
                project_id,
                if representative { MEMBERSHIP_REPRESENTATIVE } else { MEMBERSHIP_MEMBER },
                evidence,
                if evidence == EVIDENCE_DIRECT { None } else { Some(cluster.representative) },
                if evidence == EVIDENCE_DIRECT { 0i64 } else { run_id },
                detail.map(|row| row.observation_id).filter(|value: &i64| *value > 0),
                cluster.rule_version,
                detail.filter(|row| row.evidenced()).map(|row| row.observed_at.clone()).unwrap_or_default()
            ],
        )
        .map_err(|error| format!("成员写入失败：{error}"))?;
    }
    transaction.commit()?;
    Ok(id)
}

pub fn read(connection: &Connection, project_id: i64) -> Result<Vec<PersistedCluster>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id,project_id,cluster_key,kind,COALESCE(representative_asset_id,0),rule_version
             FROM exposure_clusters WHERE project_id=?1 ORDER BY cluster_key",
        )
        .map_err(|error| error.to_string())?;
    let clusters = statement
        .query_map([project_id], |row| {
            Ok(PersistedCluster {
                id: row.get(0)?,
                project_id: row.get(1)?,
                cluster_key: row.get(2)?,
                kind: row.get(3)?,
                representative_asset_id: row.get(4)?,
                rule_version: row.get(5)?,
                members: Vec::new(),
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(statement);
    let mut rows = Vec::new();
    for mut cluster in clusters {
        let mut statement = connection
            .prepare(
                "SELECT asset_id,membership,evidence_source,inherited_from_asset_id,verified_at
                 FROM exposure_cluster_members WHERE cluster_id=?1 ORDER BY asset_id",
            )
            .map_err(|error| error.to_string())?;
        cluster.members = statement
            .query_map([cluster.id], |row| {
                Ok(PersistedMember {
                    asset_id: row.get(0)?,
                    membership: row.get(1)?,
                    evidence_source: row.get(2)?,
                    inherited_from_asset_id: row.get(3)?,
                    verified_at: row.get(4)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows.push(cluster);
    }
    Ok(rows)
}

/// A revoked or removed representative makes the inherited rows unverified; the members keep
/// their own asset rows and must be observed directly again.
pub fn invalidate_inherited_from(
    connection: &Connection,
    project_id: i64,
    asset_id: i64,
) -> Result<i64, String> {
    let updated = connection
        .execute(
            "UPDATE exposure_cluster_members SET evidence_source=?3, verified_at=''
             WHERE project_id=?1 AND inherited_from_asset_id=?2 AND evidence_source=?4",
            params![project_id, asset_id, EVIDENCE_STALE, EVIDENCE_INHERITED],
        )
        .map_err(|error| error.to_string())?;
    Ok(updated as i64)
}

/// Direct observations stay direct even when the same asset also appears as a member.
pub fn direct_members(connection: &Connection, project_id: i64) -> Result<Vec<(i64, i64)>, String> {
    let mut statement = connection
        .prepare(
            "SELECT cluster_id,asset_id FROM exposure_cluster_members
             WHERE project_id=?1 AND evidence_source=?2 ORDER BY cluster_id,asset_id",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![project_id, EVIDENCE_DIRECT], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(|error| error.to_string())?;
    Ok(rows.flatten().collect())
}

pub fn kind_from_key(key: &str) -> ClusterKind {
    if key.starts_with("domain:") || key.starts_with("host:") {
        ClusterKind::Domain
    } else {
        ClusterKind::Content
    }
}

/// §E3 — the production cluster pass. Members come from rows the collector actually wrote:
/// the same bytes served from several admitted assets, or several admitted assets under one
/// registered domain. A member that lost admission is kept as a labelled gap instead of
/// quietly inheriting a conclusion it no longer earns.
const MEMBER_BASIS: &str =
    "SELECT o.asset_id, COALESCE(NULLIF(o.canonical_url,''),o.url), o.content_hash, \
     COALESCE(d.confidence,0), o.id, o.queried_at, \
     EXISTS(SELECT 1 FROM exposure_finding_observations f WHERE f.observation_id=o.id), \
     CASE WHEN COALESCE(d.exposure_eligible,0)=1 AND COALESCE(pa.is_deleted,1)=0 THEN 1 ELSE 0 END \
     FROM exposure_observations o \
     LEFT JOIN asset_ownership_decisions d ON d.asset_id=o.asset_id AND d.project_id=o.project_id \
     LEFT JOIN project_assets pa ON pa.asset_id=o.asset_id AND pa.project_id=o.project_id \
     WHERE o.project_id=?1 AND o.asset_id IS NOT NULL";

fn observed_by(
    connection: &Connection,
    project_id: i64,
    column: &str,
    value: &str,
    basis: &str,
) -> Result<Vec<ObservedMember>, String> {
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    // `column` is one of two fixed names, never caller text: the predicate cannot be widened.
    // G1-1: a domain group only takes rows drawn by the same boundary rule (`?3`), so a
    // legacy last-two-labels row or another PSL snapshot never joins a new cluster.
    let predicate = match column {
        "content_hash" => " AND o.content_hash=?2 AND o.content_hash<>'' AND ?3=?3 ORDER BY o.asset_id, o.id",
        "root_domain" => " AND o.root_domain=?2 AND o.root_domain<>'' AND o.root_basis=?3 ORDER BY o.asset_id, o.id",
        other => {
            return Err(format!(
                "exposure_cluster_basis_invalid: 只能按内容哈希或注册域取成员，收到 {other}"
            ))
        }
    };
    let sql = format!("{MEMBER_BASIS}{predicate}");
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![project_id, value, basis], |row| {
            Ok(ObservedMember {
                asset_id: row.get(0)?,
                origin: row.get(1)?,
                content_hash: row.get(2)?,
                confidence: row.get(3)?,
                observation_id: row.get(4)?,
                observed_at: row.get(5)?,
                carries_finding: row.get::<_, i64>(6)? == 1,
                admitted: row.get::<_, i64>(7)? == 1,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    // One asset may be observed by several rounds; keep its strongest row so the
    // representative rule stays a function of the data, not of insertion order.
    let mut best: std::collections::BTreeMap<i64, ObservedMember> =
        std::collections::BTreeMap::new();
    for row in rows {
        let replace = match best.get(&row.asset_id) {
            None => true,
            Some(existing) => {
                (row.confidence, -row.observation_id)
                    > (existing.confidence, -existing.observation_id)
            }
        };
        if replace {
            best.insert(row.asset_id, row);
        }
    }
    Ok(best.into_values().collect())
}

#[derive(Clone, Debug, Default)]
pub struct ClusterPass {
    pub clusters: usize,
    pub members: usize,
    /// Members that lost admission while the cluster was being written.
    pub dropped_members: i64,
}

/// Persists the clusters touched by the observations this work item just wrote. Called
/// between the observation commit and the item's terminal state, so a crash in the middle
/// leaves the item un-closed rather than a coverage claim with no graph behind it.
pub fn persist_for_observations(
    connection: &Connection,
    run_id: i64,
    project_id: i64,
    observation_ids: &[i64],
    settings_version: i64,
) -> Result<ClusterPass, String> {
    let mut pass = ClusterPass::default();
    let mut hashes: Vec<String> = Vec::new();
    let mut domains: Vec<(String, String)> = Vec::new();
    for id in observation_ids {
        let basis: Option<(Option<i64>, String, String, String)> = connection
            .query_row(
                "SELECT asset_id, content_hash, root_domain, root_basis \
                 FROM exposure_observations WHERE id=?1 AND project_id=?2",
                params![id, project_id],
                |row| {
                    Ok((
                        row.get::<_, Option<i64>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some((asset_id, content_hash, root_domain, root_basis)) = basis else {
            // The observation is not this project's: nothing may be grouped from it.
            continue;
        };
        if asset_id.is_none() || content_hash.trim().is_empty() && root_domain.trim().is_empty() {
            continue;
        }
        if !content_hash.trim().is_empty() && !hashes.contains(&content_hash) {
            hashes.push(content_hash);
        }
        // G1-1: a legacy row (empty basis) is never re-grouped by the new pass.
        let pair = (root_domain, root_basis);
        if !pair.0.trim().is_empty() && !pair.1.is_empty() && !domains.contains(&pair) {
            domains.push(pair);
        }
    }
    for hash in hashes {
        let members = observed_by(connection, project_id, "content_hash", &hash, "")?;
        pass.save_groups(
            connection,
            project_id,
            run_id,
            settings_version,
            &members,
            false,
        )?;
    }
    for (domain, basis) in domains {
        let members = observed_by(connection, project_id, "root_domain", &domain, &basis)?;
        pass.save_groups(
            connection,
            project_id,
            run_id,
            settings_version,
            &members,
            true,
        )?;
    }
    Ok(pass)
}

impl ClusterPass {
    fn save_groups(
        &mut self,
        connection: &Connection,
        project_id: i64,
        run_id: i64,
        settings_version: i64,
        members: &[ObservedMember],
        by_domain: bool,
    ) -> Result<(), String> {
        if members.len() < 2 {
            // A singleton is not a cluster: one observation cannot prove a shared surface.
            return Ok(());
        }
        let graph: Vec<super::clusters::ClusterMember> = members
            .iter()
            .map(|row| super::clusters::ClusterMember {
                asset_id: row.asset_id,
                origin: row.origin.clone(),
                content_hash: row.content_hash.clone(),
                confidence: row.confidence,
            })
            .collect();
        let clusters = if by_domain {
            super::clusters::domain_clusters(&graph)
        } else {
            super::clusters::content_clusters(&graph)
        };
        for cluster in clusters {
            let representative_observation = members
                .iter()
                .find(|row| row.asset_id == cluster.representative)
                .map(|row| row.observation_id);
            let id = save_observed(
                connection,
                project_id,
                &cluster,
                run_id,
                settings_version,
                members,
                representative_observation,
            )?;
            self.clusters += 1;
            self.members += cluster.members.len();
            self.dropped_members += members.iter().filter(|row| !row.admitted).count() as i64;
            if id == 0 {
                return Err("exposure_cluster_write_lost: 簇行没有写入".to_string());
            }
        }
        Ok(())
    }
}

/// What the cluster view shows for one cluster. `cache_time` is empty until an analysis
/// result actually exists for those bytes, so the page never reads a blank as "cached".
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSummaryRow {
    pub id: i64,
    pub cluster_key: String,
    pub kind: String,
    pub representative_asset_id: Option<i64>,
    pub representative_label: String,
    pub representative_observation_id: Option<i64>,
    pub representative_run_id: i64,
    pub representative_observed_at: String,
    pub member_count: i64,
    pub direct_count: i64,
    pub inherited_count: i64,
    pub gap_note: String,
    pub rule_version: String,
    pub settings_version: i64,
    pub updated_at: String,
    pub cache_time: String,
    pub cache_expires_at: String,
    /// G1-1 `psl` / `exact_host` / `content_hash`; '' marks a pre-G1 last-two-labels row.
    pub boundary_basis: String,
    pub psl_version: String,
    pub boundary_label: String,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterMemberRow {
    pub asset_id: i64,
    pub label: String,
    pub membership: String,
    pub evidence_source: String,
    pub inherited_from_asset_id: Option<i64>,
    pub inherited_from_label: String,
    pub inherited_run_id: i64,
    pub observation_id: Option<i64>,
    pub rule_version: String,
    pub verified_at: String,
    /// Admission re-read at display time: a revoked member stops inheriting immediately,
    /// even while its cluster row still exists.
    pub admitted_now: bool,
    pub origin: String,
    pub content_hash: String,
}

const ASSET_LABEL: &str =
    "COALESCE(NULLIF(a.domain,''),NULLIF(a.host,''),NULLIF(a.link,''),a.ip,'')";

fn page_error(error: rusqlite::Error) -> ExposureQueryError {
    ExposureQueryError::storage(&error.to_string())
}

/// One page of clusters, ordered by id so the cursor is a keyset and not an offset window.
pub fn page_clusters(
    connection: &Connection,
    project_id: i64,
    kind: &str,
    search: &str,
    cursor: &str,
    page_size: i64,
) -> Result<super::contract::ExposurePage<ClusterSummaryRow>, ExposureQueryError> {
    use super::contract::{
        decode_id_cursor, encode_id_cursor, escape_like, ExposurePage, EXPOSURE_MAX_PAGE_SIZE,
        EXPOSURE_MIN_PAGE_SIZE,
    };
    if !(EXPOSURE_MIN_PAGE_SIZE..=EXPOSURE_MAX_PAGE_SIZE).contains(&page_size) {
        return Err(ExposureQueryError::invalid_page_size(page_size));
    }
    let kind = kind.trim().to_ascii_lowercase();
    if !(kind.is_empty() || kind == "content" || kind == "domain") {
        return Err(ExposureQueryError::new(
            "exposure_cluster_kind_invalid",
            format!("未知簇类型 {kind}，已拒绝而不是当作无筛选"),
        ));
    }
    let after_id = if cursor.trim().is_empty() {
        0i64
    } else {
        decode_id_cursor(cursor)?
    };
    let pattern = if search.trim().is_empty() {
        String::new()
    } else {
        escape_like(search.trim())
    };
    // The filtered total never includes the cursor: page two reports the same whole-filter
    // count as page one, otherwise "12 clusters" would shrink as the user reads.
    let filter = "WHERE c.project_id=?1 \
         AND (?2='' OR c.kind=?2) \
         AND (?3='' OR c.cluster_key LIKE ?3 ESCAPE '\\' OR EXISTS( \
              SELECT 1 FROM exposure_cluster_members m LEFT JOIN assets s ON s.id=m.asset_id \
              WHERE m.cluster_id=c.id AND s.link LIKE ?3 ESCAPE '\\' ))";
    let cursor_tail = " AND c.id>?4";
    let total: i64 = connection
        .query_row(
            &format!("SELECT COUNT(*) FROM exposure_clusters c {filter}"),
            params![project_id, kind, pattern],
            |row| row.get(0),
        )
        .map_err(page_error)?;
    let predicate = &format!("{filter}{cursor_tail}");
    let sql = format!(
        "SELECT c.id,c.cluster_key,c.kind,c.representative_asset_id,{ASSET_LABEL},\
           c.representative_observation_id,c.representative_run_id,\
           COALESCE((SELECT o.queried_at FROM exposure_observations o WHERE o.id=c.representative_observation_id),''),\
           c.member_count,c.direct_count,c.inherited_count,c.gap_note,c.rule_version,c.settings_version,c.updated_at,\
           c.boundary_basis,c.psl_version \
         FROM exposure_clusters c \
         LEFT JOIN assets a ON a.id=c.representative_asset_id {predicate} \
         ORDER BY c.id LIMIT ?5"
    );
    let mut statement = connection.prepare(&sql).map_err(page_error)?;
    let fetched = statement
        .query_map(
            params![project_id, kind, pattern, after_id, page_size + 1],
            |row| {
                Ok(ClusterSummaryRow {
                    id: row.get(0)?,
                    cluster_key: row.get(1)?,
                    kind: row.get(2)?,
                    representative_asset_id: row.get::<_, Option<i64>>(3)?,
                    representative_label: row.get(4)?,
                    representative_observation_id: row.get(5)?,
                    representative_run_id: row.get(6)?,
                    representative_observed_at: row.get(7)?,
                    member_count: row.get(8)?,
                    direct_count: row.get(9)?,
                    inherited_count: row.get(10)?,
                    gap_note: row.get(11)?,
                    rule_version: row.get(12)?,
                    settings_version: row.get(13)?,
                    updated_at: row.get(14)?,
                    cache_time: String::new(),
                    cache_expires_at: String::new(),
                    boundary_label: super::clusters::boundary_label(
                        &row.get::<_, String>(2)?,
                        &row.get::<_, String>(15)?,
                        &row.get::<_, String>(16)?,
                    ),
                    boundary_basis: row.get(15)?,
                    psl_version: row.get(16)?,
                })
            },
        )
        .map_err(page_error)?;
    let mut items = fetched.collect::<Result<Vec<_>, _>>().map_err(page_error)?;
    let has_more = items.len() > page_size as usize;
    items.truncate(page_size as usize);
    attach_cache_times(connection, project_id, &mut items)?;
    Ok(ExposurePage {
        next_cursor: items
            .last()
            .map(|row| encode_id_cursor(row.id))
            .unwrap_or_default(),
        has_more,
        total,
        page_size,
        items,
    })
}

/// The newest cached analysis for the bytes a content cluster groups, if one exists.
fn attach_cache_times(
    connection: &Connection,
    project_id: i64,
    items: &mut [ClusterSummaryRow],
) -> Result<(), ExposureQueryError> {
    let hashes: Vec<String> = items
        .iter()
        .filter(|row| row.kind == "content")
        .map(|row| row.cluster_key.trim_start_matches("content:").to_string())
        .filter(|row| !row.is_empty())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    if hashes.is_empty() {
        return Ok(());
    }
    let marks = vec!["?"; hashes.len()].join(",");
    let sql = format!(
        "SELECT content_hash, MAX(updated_at), MAX(expires_at) FROM exposure_analysis_cache \
         WHERE project_id=?1 AND content_hash IN ({marks}) GROUP BY content_hash"
    );
    let mut statement = connection.prepare(&sql).map_err(page_error)?;
    let mut params_vec: Vec<&dyn rusqlite::ToSql> = vec![&project_id];
    for hash in &hashes {
        params_vec.push(hash);
    }
    let rows = statement
        .query_map(rusqlite::params_from_iter(params_vec), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(page_error)?;
    let mut found: std::collections::BTreeMap<String, (String, String)> =
        std::collections::BTreeMap::new();
    for row in rows {
        let (hash, updated, expires) = row.map_err(page_error)?;
        found.insert(hash, (updated, expires));
    }
    for item in items.iter_mut() {
        let hash = item.cluster_key.trim_start_matches("content:").to_string();
        if let Some((updated, expires)) = found.get(&hash) {
            item.cache_time = updated.clone();
            item.cache_expires_at = expires.clone();
        }
    }
    Ok(())
}

/// One page of a cluster's members. The list is read from the graph plus the current
/// admission, never by pulling every asset into the front end.
pub fn page_cluster_members(
    connection: &Connection,
    project_id: i64,
    cluster_id: i64,
    cursor: &str,
    page_size: i64,
) -> Result<super::contract::ExposurePage<ClusterMemberRow>, ExposureQueryError> {
    use super::contract::{
        decode_id_cursor, encode_id_cursor, ExposurePage, EXPOSURE_MAX_PAGE_SIZE,
        EXPOSURE_MIN_PAGE_SIZE,
    };
    if !(EXPOSURE_MIN_PAGE_SIZE..=EXPOSURE_MAX_PAGE_SIZE).contains(&page_size) {
        return Err(ExposureQueryError::invalid_page_size(page_size));
    }
    let owned: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_clusters WHERE id=?1 AND project_id=?2",
            params![cluster_id, project_id],
            |row| row.get(0),
        )
        .map_err(page_error)?;
    if owned == 0 {
        return Err(ExposureQueryError::new(
            "exposure_cluster_not_found",
            "这个簇不属于当前工作空间或不存在，不能按空列表处理",
        ));
    }
    let after_id = if cursor.trim().is_empty() {
        0i64
    } else {
        decode_id_cursor(cursor)?
    };
    let total: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_cluster_members WHERE cluster_id=?1 AND project_id=?2",
            params![cluster_id, project_id],
            |row| row.get(0),
        )
        .map_err(page_error)?;
    let sql = format!(
        "SELECT m.asset_id,{ASSET_LABEL},m.membership,m.evidence_source,m.inherited_from_asset_id,\
           COALESCE(NULLIF(i.domain,''),NULLIF(i.host,''),NULLIF(i.link,''),i.ip,''),\
           m.inherited_run_id,m.observation_id,m.rule_version,m.verified_at,\
           CASE WHEN COALESCE(d.exposure_eligible,0)=1 AND COALESCE(pa.is_deleted,1)=0 THEN 1 ELSE 0 END,\
           COALESCE(NULLIF(o.canonical_url,''),o.url,''),COALESCE(o.content_hash,'') \
         FROM exposure_cluster_members m \
         LEFT JOIN assets a ON a.id=m.asset_id \
         LEFT JOIN assets i ON i.id=m.inherited_from_asset_id \
         LEFT JOIN asset_ownership_decisions d ON d.asset_id=m.asset_id AND d.project_id=m.project_id \
         LEFT JOIN project_assets pa ON pa.asset_id=m.asset_id AND pa.project_id=m.project_id \
         LEFT JOIN exposure_observations o ON o.id=m.observation_id \
         WHERE m.cluster_id=?1 AND m.project_id=?2 AND m.asset_id>?3 \
         ORDER BY m.asset_id LIMIT ?4"
    );
    let mut statement = connection.prepare(&sql).map_err(page_error)?;
    let fetched = statement
        .query_map(
            params![cluster_id, project_id, after_id, page_size + 1],
            |row| {
                Ok(ClusterMemberRow {
                    asset_id: row.get(0)?,
                    label: row.get(1)?,
                    membership: row.get(2)?,
                    evidence_source: row.get(3)?,
                    inherited_from_asset_id: row.get(4)?,
                    inherited_from_label: row.get(5)?,
                    inherited_run_id: row.get(6)?,
                    observation_id: row.get(7)?,
                    rule_version: row.get(8)?,
                    verified_at: row.get(9)?,
                    admitted_now: row.get::<_, i64>(10)? == 1,
                    origin: row.get(11)?,
                    content_hash: row.get(12)?,
                })
            },
        )
        .map_err(page_error)?;
    let mut items = fetched.collect::<Result<Vec<_>, _>>().map_err(page_error)?;
    let has_more = items.len() > page_size as usize;
    items.truncate(page_size as usize);
    Ok(ExposurePage {
        next_cursor: items
            .last()
            .map(|row| encode_id_cursor(row.asset_id))
            .unwrap_or_default(),
        has_more,
        total,
        page_size,
        items,
    })
}
