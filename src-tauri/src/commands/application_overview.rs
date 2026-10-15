//! Read-only application overview across project-owned records.
//! Aggregation belongs to the shell rather than to a specific business module.
use crate::{db, models::DashboardStats, AppState};
use tauri::State;

#[tauri::command]
pub async fn dashboard_stats(
    state: State<'_, AppState>,
    project_id: Option<i64>,
) -> Result<DashboardStats, String> {
    let connection = db::open(&state.db_path)?;
    let project_count = connection
        .query_row(
            "SELECT COUNT(*) FROM projects WHERE status='active'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let running_jobs = connection
        .query_row(
            "SELECT COUNT(*) FROM runs WHERE status IN ('queued','running','cancel_requested')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let (asset_count, alive_count, pending_count, blocked_count): (i64, i64, i64, i64) = connection
        .query_row(
            r#"SELECT COUNT(*),
                      COALESCE(SUM(CASE WHEN a.probe_outcome IN ('web_alive','web_restricted','browser_render_required','virtual_host_required') AND COALESCE(a.content_category,'') NOT IN ('gambling','porn','custom_rule') THEN 1 ELSE 0 END),0),
                      COALESCE(SUM(CASE WHEN pa.decision IN ('pending','uncertain','') THEN 1 ELSE 0 END),0),
                      COALESCE(SUM(CASE WHEN a.probe_outcome='blocked_content' OR a.content_category IN ('gambling','porn','custom_rule') THEN 1 ELSE 0 END),0)
               FROM project_assets pa JOIN assets a ON a.id=pa.asset_id
               WHERE pa.is_deleted=0 AND (?1 IS NULL OR pa.project_id=?1)"#,
            [project_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|error| error.to_string())?;
    let page_blocked: i64 = connection
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM project_assets pa WHERE (?1 IS NULL OR pa.project_id=?1) AND {}",
                crate::page_isolation::owned_isolation_sql()
            ),
            [project_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let blocked_count = blocked_count + page_blocked;
    let (new_count, changed_count) = connection
        .query_row(
            r#"SELECT COALESCE(SUM(CASE WHEN event_type='new' THEN 1 ELSE 0 END),0),
                      COALESCE(SUM(CASE WHEN event_type='changed' THEN 1 ELSE 0 END),0)
               FROM asset_events
               WHERE created_at>=datetime('now','-30 day') AND (?1 IS NULL OR project_id=?1)"#,
            [project_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| error.to_string())?;
    Ok(DashboardStats {
        project_count,
        asset_count,
        alive_count,
        pending_count,
        new_count,
        changed_count,
        blocked_count,
        running_jobs,
    })
}
