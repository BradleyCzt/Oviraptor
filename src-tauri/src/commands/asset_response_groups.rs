use super::{field_sql, ASSET_SELECT};
use crate::asset_models::AssetQuery;

// Presentation groups never merge/delete assets or grant member-wide decisions.
// Only the complete-response proof lives in global technical metadata; project
// membership and representatives are derived after every query's full filter.

const RESPONSE_GROUP_PARTITION: &str = "project_id,response_key,response_status,response_hash,response_bytes,response_content_type,
    decision_state,note_state,trash_state,entry_seen_state,sentinel_state,sentinel_count,
    ownership_state,ownership_confidence,authorization_state,exposure_state,ownership_source,ownership_reason,
    login_state,register_state";

fn asset_response_json_sql(field: &str) -> String {
    format!("json_extract(CASE WHEN json_valid(a.extra_json) THEN a.extra_json ELSE '{{}}' END,'$.{field}')")
}

pub(super) fn asset_response_fingerprint_sql() -> String {
    let fingerprint = asset_response_json_sql("probe_response_fingerprint");
    let hash = asset_response_json_sql("body_sha256");
    let bytes = asset_response_json_sql("probe_body_bytes");
    let limit = asset_response_json_sql("probe_body_limit");
    let complete = asset_response_json_sql("probe_body_complete");
    let truncated = asset_response_json_sql("probe_body_truncated");
    let outcome = asset_response_json_sql("probe_outcome");
    let status = asset_response_json_sql("probe_status_code");
    let encoding = asset_response_json_sql("probe_content_encoding");
    let range = asset_response_json_sql("probe_content_range");
    let error = asset_response_json_sql("probe_error");
    format!(
        r#"CASE WHEN a.probe_outcome='web_alive'
        AND a.status_code GLOB '[0-9][0-9][0-9]'
        AND CAST(a.status_code AS INTEGER) BETWEEN 200 AND 299 AND a.status_code<>'206'
        AND {outcome}='web_alive' AND {status}=a.status_code
        AND {complete}='true' AND {truncated}='false'
        AND COALESCE({error},'')='' AND COALESCE({range},'')=''
        AND lower(COALESCE({encoding},'')) IN ('','identity')
        AND length({fingerprint})=64 AND {fingerprint} NOT GLOB '*[^0-9a-fA-F]*'
        AND length({hash})=64 AND {hash} NOT GLOB '*[^0-9a-fA-F]*'
        AND CAST({bytes} AS TEXT)<>'' AND CAST({bytes} AS TEXT) NOT GLOB '*[^0-9]*'
        AND CAST({limit} AS TEXT)<>'' AND CAST({limit} AS TEXT) NOT GLOB '*[^0-9]*'
        AND CAST({bytes} AS INTEGER)>0 AND CAST({bytes} AS INTEGER)<=CAST({limit} AS INTEGER)
        THEN {fingerprint} ELSE NULL END"#
    )
}

pub(super) fn validate_asset_response_query(query: &AssetQuery) -> Result<(), String> {
    let fingerprint = query.response_fingerprint.trim();
    if let Some(representative) = query.response_representative_asset_id {
        if representative <= 0 {
            return Err("响应组代表资产 ID 必须为正数".into());
        }
        if query.project_id.is_none() || fingerprint.is_empty() {
            return Err("查看精确响应组时必须指定项目和完整响应指纹".into());
        }
    }
    if !fingerprint.is_empty() {
        if query.project_id.is_none() {
            return Err("查看同响应成员时必须指定项目".into());
        }
        if fingerprint.len() != 64 || !fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("完整响应指纹格式无效".into());
        }
    }
    Ok(())
}

pub(super) fn asset_response_group_cte(filter: &str) -> String {
    let fingerprint = asset_response_fingerprint_sql();
    let hash = asset_response_json_sql("body_sha256");
    let bytes = asset_response_json_sql("probe_body_bytes");
    let content_type = asset_response_json_sql("probe_content_type");
    let sentinel_status = field_sql("sentinelStatus").expect("known asset field");
    let sentinel_count = field_sql("sentinelScanCount").expect("known asset field");
    // Include the exact response tuple as well as its fingerprint. A mismatched
    // or reused fingerprint cannot fold different hashes/lengths/MIME types.
    let partition = RESPONSE_GROUP_PARTITION;
    format!(
        r#"WITH response_matched AS (
        SELECT a.id AS asset_id,pa.project_id,
          COALESCE(({fingerprint}),'asset:'||a.id) AS response_key,
          a.status_code AS response_status,
          COALESCE({hash},'') AS response_hash,COALESCE({bytes},'') AS response_bytes,
          COALESCE({content_type},'') AS response_content_type,
          pa.decision AS decision_state,pa.note AS note_state,pa.is_deleted AS trash_state,
          CASE WHEN COALESCE(pa.entry_opened_at,'')='' THEN 0 ELSE 1 END AS entry_seen_state,
          {sentinel_status} AS sentinel_state,{sentinel_count} AS sentinel_count,
          COALESCE(o.status,'unreviewed') AS ownership_state,COALESCE(o.confidence,0) AS ownership_confidence,
          COALESCE(o.authorization_status,'unknown') AS authorization_state,
          COALESCE(o.exposure_eligible,0) AS exposure_state,COALESCE(o.source,'') AS ownership_source,
          COALESCE(o.reason,'') AS ownership_reason,a.has_login AS login_state,a.has_register AS register_state
        FROM assets a JOIN project_assets pa ON pa.asset_id=a.id JOIN projects p ON p.id=pa.project_id
        LEFT JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=a.id
        WHERE {filter}
      ),response_ranked AS (
        SELECT *,ROW_NUMBER() OVER(PARTITION BY {partition} ORDER BY asset_id) AS response_display_rank,
          COUNT(*) OVER(PARTITION BY {partition}) AS response_display_group_count
        FROM response_matched
      )"#
    )
}

pub(super) fn asset_response_member_filter_sql(filter: &str) -> String {
    // Derive both the anchor and members from the same fully filtered snapshot.
    // IS is null-safe, matching the window partition's treatment of NULL values.
    let matching_partition = RESPONSE_GROUP_PARTITION
        .split(',')
        .map(str::trim)
        .map(|field| format!("member.{field} IS anchor.{field}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    format!(
        "a.id IN ({} SELECT member.asset_id FROM response_ranked member JOIN response_ranked anchor ON {matching_partition} WHERE anchor.asset_id=?)",
        asset_response_group_cte(filter)
    )
}

pub(super) fn asset_response_list_sql(filter: &str, order: &str, collapse: bool) -> String {
    // Keep ASSET_SELECT and its raw export contract unchanged. The additional
    // final column is only read by list_assets_on and never persisted.
    let marker = "a.has_login,a.has_register,pa.entry_opened_at";
    let count = if collapse {
        "response_display.response_display_group_count"
    } else {
        "1"
    };
    let select = ASSET_SELECT.replacen(marker, &format!("{marker},{count}"), 1);
    if collapse {
        format!("{} {select} JOIN response_ranked response_display ON response_display.asset_id=a.id AND response_display.project_id=pa.project_id WHERE response_display.response_display_rank=1 ORDER BY {order} LIMIT ? OFFSET ?", asset_response_group_cte(filter))
    } else {
        format!("{select} WHERE {filter} ORDER BY {order} LIMIT ? OFFSET ?")
    }
}

#[cfg(test)]
#[path = "asset_response_groups_tests.rs"]
mod asset_response_groups_tests;
