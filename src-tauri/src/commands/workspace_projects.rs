//! Project identity and lifecycle. Cross-business counts are deletion guards;
//! this module does not initialize assets, scans, chat, or model execution.
use crate::{db, models::{Project, ProjectImpact, ProjectInput}, AppState};
use rusqlite::params;
use tauri::State;

#[tauri::command]
pub async fn list_projects(state: State<'_, AppState>) -> Result<Vec<Project>, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement = connection.prepare(r#"
        SELECT p.id,p.name,p.description,p.status,p.created_at,p.updated_at,
               (SELECT COUNT(*) FROM project_assets pa WHERE pa.project_id=p.id AND pa.is_deleted=0),
               (SELECT COUNT(*) FROM project_assets pa WHERE pa.project_id=p.id AND pa.is_deleted=0 AND pa.decision IN ('pending','uncertain','')),
               (SELECT COUNT(*) FROM targets t WHERE t.project_id=p.id),
               (SELECT COUNT(*) FROM runs r WHERE r.project_id=p.id),
               (SELECT COUNT(*) FROM sentinel_scans s WHERE s.project_id=p.id),
               (SELECT COUNT(*) FROM sentinel_findings f JOIN sentinel_scans s ON s.id=f.scan_id WHERE s.project_id=p.id AND f.kind='vulnerability'),
               (SELECT COUNT(*) FROM sentinel_validations v JOIN sentinel_scans s ON s.id=v.scan_id WHERE s.project_id=p.id AND v.verdict<>'pending'),
               (SELECT COUNT(*) FROM sentinel_fuse_zone z WHERE z.project_id=p.id AND z.archived=0),
               (SELECT MAX(COALESCE(r.finished_at,r.created_at)) FROM runs r WHERE r.project_id=p.id),
               (SELECT MAX(s.updated_at) FROM sentinel_scans s WHERE s.project_id=p.id)
        FROM projects p ORDER BY p.updated_at DESC, p.id DESC
    "#).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(Project {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                status: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
                asset_count: row.get(6)?,
                pending_count: row.get(7)?,
                target_count: row.get(8)?,
                asset_run_count: row.get(9)?,
                scan_count: row.get(10)?,
                vulnerability_count: row.get(11)?,
                validation_count: row.get(12)?,
                active_fuse_count: row.get(13)?,
                last_run_at: row.get(14)?,
                last_scan_at: row.get(15)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

// Keep explicit UI categories, but do not let a newly added project-owned table
// silently become deletable. Include conventional project_id columns even when
// legacy schemas lack an FK, and differently named direct FKs to projects.id.
fn project_other_record_count(
    connection: &rusqlite::Connection,
    project_id: i64,
) -> Result<i64, String> {
    let categorized = [
        "project_assets", "asset_events", "targets", "runs", "saved_views",
        "sentinel_scans", "sentinel_targets", "sentinel_opportunities",
        "sentinel_fuse_zone", "appsec_vulnerabilities", "agent_knowledge_entries",
        "agent_learning_candidates", "browser_auth_sessions",
    ];
    let mut statement = connection.prepare(
        "SELECT m.name,c.name FROM sqlite_schema m JOIN pragma_table_info(m.name) c \
         WHERE m.type='table' AND m.name NOT LIKE 'sqlite_%' AND \
         (c.name='project_id' OR EXISTS(SELECT 1 FROM pragma_foreign_key_list(m.name) f \
         WHERE f.\"table\"='projects' AND f.\"from\"=c.name AND (f.\"to\"='id' OR f.\"to\" IS NULL))) \
         ORDER BY m.name,c.cid",
    ).map_err(|error| error.to_string())?;
    let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|error| error.to_string())?;
    let mut tables = std::collections::BTreeMap::<String, Vec<String>>::new();
    for row in rows {
        let (table, column) = row.map_err(|error| error.to_string())?;
        if column == "project_id" && categorized.contains(&table.as_str()) {
            continue;
        }
        tables.entry(table).or_default().push(column);
    }
    let mut total = 0_i64;
    for (table, columns) in tables {
        let predicate = columns.iter().map(|column| format!("\"{}\"=?1", column.replace('"', "\"\"")))
            .collect::<Vec<_>>().join(" OR ");
        let count: i64 = connection.query_row(
            &format!("SELECT COUNT(*) FROM \"{}\" WHERE {predicate}", table.replace('"', "\"\"")),
            [project_id], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        total = total.checked_add(count).ok_or("项目关联记录计数溢出")?;
    }
    Ok(total)
}

pub(super) fn project_impact_for_connection(
    connection: &rusqlite::Connection,
    project_id: i64,
) -> Result<ProjectImpact, String> {
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
            [project_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !exists {
        return Err("工作空间不存在或已被删除".into());
    }
    let count = |sql: &str| -> Result<i64, String> {
        connection
            .query_row(sql, [project_id], |row| row.get(0))
            .map_err(|error| error.to_string())
    };
    let asset_count = count("SELECT COUNT(*) FROM project_assets WHERE project_id=?1")?;
    let asset_event_count = count("SELECT COUNT(*) FROM asset_events WHERE project_id=?1")?;
    let target_count = count("SELECT COUNT(*) FROM targets WHERE project_id=?1")?;
    let asset_run_count = count("SELECT COUNT(*) FROM runs WHERE project_id=?1")?;
    let saved_view_count = count("SELECT COUNT(*) FROM saved_views WHERE project_id=?1")?;
    let sentinel_scan_count = count("SELECT COUNT(*) FROM sentinel_scans WHERE project_id=?1")?;
    let sentinel_target_count = count("SELECT COUNT(*) FROM sentinel_targets WHERE project_id=?1")?;
    let finding_count = count("SELECT COUNT(*) FROM sentinel_findings f JOIN sentinel_scans s ON s.id=f.scan_id WHERE s.project_id=?1")?;
    let validation_count = count("SELECT COUNT(*) FROM sentinel_validations v JOIN sentinel_scans s ON s.id=v.scan_id WHERE s.project_id=?1")?;
    let opportunity_count =
        count("SELECT COUNT(*) FROM sentinel_opportunities WHERE project_id=?1")?;
    let fuse_count = count("SELECT COUNT(*) FROM sentinel_fuse_zone WHERE project_id=?1")?;
    let appsec_vulnerability_count =
        count("SELECT COUNT(*) FROM appsec_vulnerabilities WHERE project_id=?1")?;
    let knowledge_count =
        count("SELECT COUNT(*) FROM agent_knowledge_entries WHERE project_id=?1")?;
    let learning_candidate_count =
        count("SELECT COUNT(*) FROM agent_learning_candidates WHERE project_id=?1")?;
    let browser_auth_session_count =
        count("SELECT COUNT(*) FROM browser_auth_sessions WHERE project_id=?1")?;
    let other_record_count = project_other_record_count(connection, project_id)?;
    let total_records = asset_count
        + asset_event_count
        + target_count
        + asset_run_count
        + saved_view_count
        + sentinel_scan_count
        + sentinel_target_count
        + finding_count
        + validation_count
        + opportunity_count
        + fuse_count
        + appsec_vulnerability_count
        + knowledge_count
        + learning_candidate_count
        + browser_auth_session_count
        + other_record_count;
    Ok(ProjectImpact {
        asset_count,
        asset_event_count,
        target_count,
        asset_run_count,
        saved_view_count,
        sentinel_scan_count,
        sentinel_target_count,
        finding_count,
        validation_count,
        opportunity_count,
        fuse_count,
        appsec_vulnerability_count,
        knowledge_count,
        learning_candidate_count,
        browser_auth_session_count,
        other_record_count,
        total_records,
    })
}

#[tauri::command]
pub fn project_impact(
    state: State<'_, AppState>,
    project_id: i64,
) -> Result<ProjectImpact, String> {
    let mut connection = db::open(&state.db_path)?;
    let transaction = connection.transaction().map_err(|error| error.to_string())?;
    let impact = project_impact_for_connection(&transaction, project_id)?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(impact)
}

#[tauri::command]
pub fn save_project(state: State<AppState>, input: ProjectInput) -> Result<i64, String> {
    let connection = db::open(&state.db_path)?;
    let name = input.name.trim();
    if name.is_empty() {
        return Err("项目名称不能为空".into());
    }
    let duplicate: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE lower(trim(name))=lower(?1) AND (?2 IS NULL OR id<>?2))",
            params![name, input.id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if duplicate {
        return Err(format!("项目名称“{name}”已存在，请使用其他名称"));
    }
    if let Some(id) = input.id {
        let updated = connection.execute(
            "UPDATE projects SET name=?1,description=?2,updated_at=datetime('now','localtime') WHERE id=?3",
            params![name, input.description.trim(), id],
        ).map_err(|error| error.to_string())?;
        if updated == 0 {
            return Err("工作空间不存在或已被删除".into());
        }
        Ok(id)
    } else {
        connection
            .execute(
                "INSERT INTO projects(name,description) VALUES(?1,?2)",
                params![name, input.description.trim()],
            )
            .map_err(|error| error.to_string())?;
        Ok(connection.last_insert_rowid())
    }
}

#[tauri::command]
pub fn archive_project(
    state: State<'_, AppState>,
    project_id: i64,
    archived: bool,
) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    let updated = connection
        .execute(
            "UPDATE projects SET status=?1,updated_at=datetime('now','localtime') WHERE id=?2",
            params![if archived { "archived" } else { "active" }, project_id],
        )
        .map_err(|error| error.to_string())?;
    if updated == 0 {
        return Err("工作空间不存在或已被删除".into());
    }
    Ok(())
}

#[tauri::command]
pub fn delete_project(state: State<AppState>, project_id: i64) -> Result<(), String> {
    let mut connection = db::open(&state.db_path)?;
    delete_project_for_connection(&mut connection, project_id, || {})
}

// The callback is a deterministic concurrency test seam; the command passes a no-op.
pub(super) fn delete_project_for_connection(
    connection: &mut rusqlite::Connection,
    project_id: i64,
    after_admission: impl FnOnce(),
) -> Result<(), String> {
    connection.pragma_update(None, "synchronous", "FULL").map_err(|error| error.to_string())?;
    // Reserve the writer before any admission read. A UI preview is advisory;
    // only this transaction can authorize deleting an actually empty project.
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let impact = project_impact_for_connection(&transaction, project_id)?;
    if impact.total_records > 0 {
        return Err(format!(
            "该工作空间仍关联 {} 条记录（{} 条资产、{} 个扫描任务、{} 条证据/结论），不能删除；请归档以保留完整历史",
            impact.total_records,
            impact.asset_count,
            impact.sentinel_scan_count,
            impact.finding_count + impact.validation_count
        ));
    }
    after_admission();
    let deleted = transaction
        .execute("DELETE FROM projects WHERE id=?1", [project_id])
        .map_err(|error| error.to_string())?;
    if deleted != 1 {
        return Err("项目删除未生效；未提交任何变更".into());
    }
    let remains: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)", [project_id], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if remains {
        return Err("项目删除后置校验失败；未提交任何变更".into());
    }
    // 项目在上方已经确认没有关联资产。这里不能顺带扫描并清理整个 assets 表：
    // 数据量较大时，删除一个空项目会退化成昂贵的全库维护操作。
    // 孤儿资产清理由独立的数据库维护流程负责。
    transaction.commit().map_err(|error| error.to_string())
}
