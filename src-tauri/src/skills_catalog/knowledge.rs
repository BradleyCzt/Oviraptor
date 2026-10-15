//! Reusable knowledge owns provenance, review and explicit application to saved
//! instructions. Scan ids are retained source identity and optional read filters;
//! this module never starts, resumes or analyzes an investigation.
use super::{
    context::agent_learning_catalog,
    instructions::portable_export_path,
    model::{AgentKnowledgeEntry, AgentLearningCandidate},
    patch::{
        apply_skill_patch, refine_learning_patch_for_apply, skill_compare_text,
        skill_patch_has_content,
    },
};
use crate::{db, AppState};
use rusqlite::{params, Row};
use serde_json::Value as JsonValue;
use std::{fs, path::Path};
use tauri::State;
pub(super) fn json(text: String) -> JsonValue {
    serde_json::from_str(&text).unwrap_or_else(|_| JsonValue::Object(Default::default()))
}

pub(super) fn knowledge_row(row: &Row<'_>) -> rusqlite::Result<AgentKnowledgeEntry> {
    Ok(AgentKnowledgeEntry {
        id: row.get(0)?,
        scan_id: row.get(1)?,
        project_id: row.get(2)?,
        title: row.get(3)?,
        summary: row.get(4)?,
        patterns: json(row.get(5)?),
        source_hash: row.get(6)?,
        skill_id: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

pub(super) const KNOWLEDGE_COLUMNS: &str =
    "id,scan_id,project_id,title,summary,patterns_json,source_hash,skill_id,created_at,updated_at";

pub(super) const LEARNING_CANDIDATE_COLUMNS: &str =
    "id,scan_id,project_id,scan_type,title,summary,candidate_json,status,target_skill_id,source_hash,created_at,reviewed_at,updated_at";

pub(super) fn learning_candidate_row(row: &Row<'_>) -> rusqlite::Result<AgentLearningCandidate> {
    Ok(AgentLearningCandidate {
        id: row.get(0)?,
        scan_id: row.get(1)?,
        project_id: row.get(2)?,
        scan_type: row.get(3)?,
        title: row.get(4)?,
        summary: row.get(5)?,
        candidate: json(row.get(6)?),
        status: row.get(7)?,
        target_skill_id: row.get(8)?,
        source_hash: row.get(9)?,
        created_at: row.get(10)?,
        reviewed_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

#[tauri::command]
pub fn list_agent_knowledge(
    state: State<AppState>,
    scan_id: Option<String>,
) -> Result<Vec<AgentKnowledgeEntry>, String> {
    let connection = db::open(&state.db_path)?;
    query_agent_knowledge(&connection, scan_id.as_deref())
}

fn query_agent_knowledge(
    connection: &rusqlite::Connection,
    scan_id: Option<&str>,
) -> Result<Vec<AgentKnowledgeEntry>, String> {
    let mut statement = connection
        .prepare(&format!("SELECT {KNOWLEDGE_COLUMNS} FROM agent_knowledge_entries WHERE (?1 IS NULL OR scan_id=?1) ORDER BY updated_at DESC,id DESC"))
        .map_err(|error| error.to_string())?;
    let entries = statement
        .query_map([scan_id], knowledge_row)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(entries)
}

#[tauri::command]
pub fn list_agent_learning_candidates(
    state: State<AppState>,
    status: Option<String>,
    scan_id: Option<String>,
) -> Result<Vec<AgentLearningCandidate>, String> {
    let connection = db::open(&state.db_path)?;
    query_agent_learning_candidates(&connection, status.as_deref(), scan_id.as_deref())
}

fn query_agent_learning_candidates(
    connection: &rusqlite::Connection,
    status: Option<&str>,
    scan_id: Option<&str>,
) -> Result<Vec<AgentLearningCandidate>, String> {
    let mut statement = connection
        .prepare(&format!("SELECT {LEARNING_CANDIDATE_COLUMNS} FROM agent_learning_candidates WHERE (?1 IS NULL OR status=?1) AND (?2 IS NULL OR scan_id=?2) ORDER BY updated_at DESC,id DESC"))
        .map_err(|error| error.to_string())?;
    let result = statement
        .query_map(params![status, scan_id], learning_candidate_row)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string());
    result
}

#[cfg(test)]
#[test]
fn task_learning_readers_filter_by_scan_without_changing_the_global_catalog() {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection.execute_batch(
        "CREATE TABLE agent_knowledge_entries(id INTEGER,scan_id TEXT,project_id INTEGER,title TEXT,summary TEXT,patterns_json TEXT,source_hash TEXT,skill_id INTEGER,created_at TEXT,updated_at TEXT); \
         CREATE TABLE agent_learning_candidates(id INTEGER,scan_id TEXT,project_id INTEGER,scan_type TEXT,title TEXT,summary TEXT,candidate_json TEXT,status TEXT,target_skill_id INTEGER,source_hash TEXT,created_at TEXT,reviewed_at TEXT,updated_at TEXT); \
         INSERT INTO agent_knowledge_entries VALUES(1,'scan-a',1,'A','summary','{}','hash-a',NULL,'t','t'),(2,'scan-b',2,'B','summary','{}','hash-b',NULL,'t','t'); \
         INSERT INTO agent_learning_candidates VALUES(1,'scan-a',1,'web','A','summary','{}','pending',NULL,'hash-a','t','','t'),(2,'scan-b',2,'web','B','summary','{}','accepted',NULL,'hash-b','t','','t');",
    ).unwrap();
    assert_eq!(query_agent_knowledge(&connection, None).unwrap().len(), 2);
    let knowledge = query_agent_knowledge(&connection, Some("scan-a")).unwrap();
    assert_eq!(knowledge.len(), 1);
    assert_eq!(knowledge[0].scan_id, "scan-a");
    assert!(query_agent_knowledge(&connection, Some("unknown"))
        .unwrap()
        .is_empty());
    assert_eq!(
        query_agent_learning_candidates(&connection, None, None)
            .unwrap()
            .len(),
        2
    );
    let candidates =
        query_agent_learning_candidates(&connection, Some("pending"), Some("scan-a")).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].scan_id, "scan-a");
    assert!(
        query_agent_learning_candidates(&connection, Some("accepted"), Some("scan-a"))
            .unwrap()
            .is_empty()
    );
}

#[tauri::command]
pub fn review_agent_learning_candidate(
    state: State<AppState>,
    candidate_id: i64,
    decision: String,
    target_skill_id: Option<i64>,
) -> Result<AgentLearningCandidate, String> {
    let decision = decision.trim().to_ascii_lowercase();
    if !["accepted", "rejected", "pending"].contains(&decision.as_str()) {
        return Err("候选审核状态必须是 accepted、rejected 或 pending".into());
    }
    let connection = db::open(&state.db_path)?;
    let changed = connection
        .execute("UPDATE agent_learning_candidates SET status=?1,target_skill_id=COALESCE(?2,target_skill_id),reviewed_at=CASE WHEN ?1='pending' THEN '' ELSE datetime('now','localtime') END,updated_at=datetime('now','localtime') WHERE id=?3", params![decision,target_skill_id,candidate_id])
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("学习候选不存在".into());
    }
    connection
        .query_row(
            &format!(
                "SELECT {LEARNING_CANDIDATE_COLUMNS} FROM agent_learning_candidates WHERE id=?1"
            ),
            [candidate_id],
            learning_candidate_row,
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_agent_learning_candidate(
    state: State<AppState>,
    candidate_id: i64,
) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    let deleted = connection
        .execute(
            "DELETE FROM agent_learning_candidates WHERE id=?1",
            [candidate_id],
        )
        .map_err(|error| error.to_string())?;
    if deleted == 0 {
        return Err("学习候选不存在".into());
    }
    Ok(())
}

#[tauri::command]
pub fn apply_agent_learning_candidate(
    state: State<AppState>,
    candidate_id: i64,
) -> Result<i64, String> {
    let connection = db::open(&state.db_path)?;
    let (status, target_skill_id, candidate_json, scan_id): (String, Option<i64>, String, String) = connection
        .query_row("SELECT status,target_skill_id,candidate_json,scan_id FROM agent_learning_candidates WHERE id=?1", [candidate_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)))
        .map_err(|_| "学习候选不存在".to_string())?;
    if status != "accepted" {
        return Err("请先审核接受该候选，再沉淀为 Skill".into());
    }
    let mut candidate = json(candidate_json);
    let quality_disposition = candidate
        .pointer("/qualityGate/disposition")
        .and_then(JsonValue::as_str)
        .unwrap_or("unknown");
    if quality_disposition != "reusable_candidate" {
        return Err(format!(
            "该候选未通过质量门禁（{quality_disposition}），请先补充复现/影响证据后再沉淀"
        ));
    }
    let original_patch = candidate.get("skillPatch").cloned().unwrap_or_default();
    let mut skill_id = target_skill_id;
    let mut name = String::new();
    let description = candidate
        .get("summary")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .to_string();
    let mut base_instructions = String::new();
    if let Some(id) = skill_id {
        let (builtin, old_name, old_instructions): (i64, String, String) = connection
            .query_row(
                "SELECT builtin,name,instructions FROM agent_skills WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| "目标 Skill 不存在".to_string())?;
        if builtin != 0 {
            skill_id = None;
            name = format!("{} · 增强版", old_name);
            base_instructions = old_instructions;
        } else {
            name = old_name;
            base_instructions = old_instructions;
        }
    }
    // Applying an accepted candidate is deterministic. A second model call used
    // to rewrite the already-reviewed patch here, so changing providers could
    // silently produce a different Skill and spend more tokens. Explicit
    // "refine with latest knowledge" remains available as a separate action.
    let patch = original_patch;
    if !skill_patch_has_content(&patch) {
        return Err("候选没有可应用的规范化 Markdown 补丁".into());
    }
    let refinement_status = "reviewed_canonical_patch";
    if skill_id.is_none() {
        if let Some(existing_id) = patch.get("targetSkillId").and_then(JsonValue::as_i64) {
            if let Ok((builtin, old_name, old_instructions)) = connection.query_row(
                "SELECT builtin,name,instructions FROM agent_skills WHERE id=?1",
                [existing_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            ) {
                if builtin == 0 {
                    skill_id = Some(existing_id);
                    name = old_name;
                    base_instructions = old_instructions;
                }
            }
        }
    }
    if let Some(object) = candidate.as_object_mut() {
        object.insert("skillPatch".into(), patch.clone());
        object.insert(
            "applyRefinement".into(),
            JsonValue::String(refinement_status.into()),
        );
    }
    let instructions = apply_skill_patch(&base_instructions, &patch);
    if instructions.is_empty() {
        return Err("候选没有可沉淀的 Skill 补丁，请重新生成或补充候选".into());
    }
    let normalized = skill_compare_text(&instructions);
    if skill_id.is_none() {
        let duplicate = connection
            .prepare("SELECT id,instructions FROM agent_skills")
            .ok()
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                    })
                    .ok()
                    .and_then(|rows| {
                        rows.flatten()
                            .find(|(_, existing)| skill_compare_text(existing) == normalized)
                    })
            });
        if let Some((existing_id, _)) = duplicate {
            skill_id = Some(existing_id);
        }
    }
    if name.is_empty() {
        name = format!(
            "扫描学习 · {}",
            candidate
                .get("title")
                .and_then(JsonValue::as_str)
                .unwrap_or(&scan_id)
        );
    }
    let name = name.chars().take(80).collect::<String>();
    if skill_id.is_none() {
        connection.execute("INSERT INTO agent_skills(name,description,instructions,builtin,enabled) VALUES(?1,?2,?3,0,1)", params![name,description,instructions]).map_err(|error| error.to_string())?;
        skill_id = Some(connection.last_insert_rowid());
    } else {
        connection.execute("UPDATE agent_skills SET instructions=?1,updated_at=datetime('now','localtime') WHERE id=?2 AND builtin=0", params![instructions,skill_id]).map_err(|error| error.to_string())?;
    }
    let skill_id = skill_id.ok_or("无法创建 Skill")?;
    connection.execute("UPDATE agent_learning_candidates SET status='applied',target_skill_id=?1,candidate_json=?2,reviewed_at=COALESCE(NULLIF(reviewed_at,''),datetime('now','localtime')),updated_at=datetime('now','localtime') WHERE id=?3", params![skill_id,candidate.to_string(),candidate_id]).map_err(|error| error.to_string())?;
    Ok(skill_id)
}

#[tauri::command]
pub fn delete_agent_knowledge(state: State<AppState>, knowledge_id: i64) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    let deleted = connection
        .execute(
            "DELETE FROM agent_knowledge_entries WHERE id=?1",
            [knowledge_id],
        )
        .map_err(|error| error.to_string())?;
    if deleted == 0 {
        return Err("知识条目不存在".into());
    }
    Ok(())
}

#[tauri::command]
pub fn convert_agent_knowledge_to_instruction(
    state: State<AppState>,
    knowledge_id: i64,
) -> Result<i64, String> {
    let connection = db::open(&state.db_path)?;
    let (title, summary, instructions, patterns_json, linked_skill): (String, String, String, String, Option<i64>) = connection
        .query_row(
            "SELECT title,summary,skill_instructions,patterns_json,skill_id FROM agent_knowledge_entries WHERE id=?1",
            [knowledge_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .map_err(|_| "知识条目不存在".to_string())?;
    if let Some(skill_id) = linked_skill {
        return Ok(skill_id);
    }
    let patterns = json(patterns_json);
    let quality_score = patterns
        .get("qualityScore")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0);
    let knowledge_kind = patterns
        .get("knowledgeKind")
        .and_then(JsonValue::as_str)
        .unwrap_or("");
    if knowledge_kind.is_empty() {
        return Err(
            "这是旧版未评分知识，请先从轨迹重新生成候选或执行同类聚合，再转换为 Skill".into(),
        );
    }
    if quality_score < 70 {
        return Err(format!(
            "知识质量 {quality_score}/100，低于转换 Skill 所需的 70 分；请补充证据或使用多任务聚合"
        ));
    }

    // A scored knowledge entry is already a deterministic, target-neutral
    // method card. Conversion must not vary with the model provider currently
    // configured, nor spend another refinement request.
    let refined = serde_json::json!({
        "addSections": [{"title": "沉淀知识", "content": instructions}],
        "decision": "create_new",
        "reasoning": "deterministic knowledge conversion"
    });
    let mut target_skill_id = refined.get("targetSkillId").and_then(JsonValue::as_i64);
    let mut base_instructions = String::new();
    let mut name = format!("知识 · {}", title)
        .chars()
        .take(72)
        .collect::<String>();
    if let Some(id) = target_skill_id {
        if let Ok((builtin, old_name, old_instructions)) = connection.query_row(
            "SELECT builtin,name,instructions FROM agent_skills WHERE id=?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        ) {
            if builtin == 0 {
                name = old_name;
                base_instructions = old_instructions;
            } else {
                target_skill_id = None;
                name = format!("{} · 增强版", old_name);
                base_instructions = old_instructions;
            }
        } else {
            target_skill_id = None;
        }
    }
    let merged = apply_skill_patch(&base_instructions, &refined);
    if merged.trim().is_empty() {
        return Err("全局去重后没有可沉淀的 Skill 内容".into());
    }
    let normalized = skill_compare_text(&merged);
    if target_skill_id.is_none() {
        let duplicate = connection
            .prepare("SELECT id,instructions FROM agent_skills")
            .ok()
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                    })
                    .ok()
                    .and_then(|rows| {
                        rows.flatten()
                            .find(|(_, existing)| skill_compare_text(existing) == normalized)
                    })
            });
        target_skill_id = duplicate.map(|(id, _)| id);
    }
    let skill_id = if let Some(id) = target_skill_id {
        connection.execute("UPDATE agent_skills SET instructions=?1,description=CASE WHEN trim(description)='' THEN ?2 ELSE description END,updated_at=datetime('now','localtime') WHERE id=?3 AND builtin=0", params![merged,summary,id]).map_err(|error| error.to_string())?;
        id
    } else {
        let mut candidate_name = name.clone();
        if connection
            .query_row(
                "SELECT COUNT(*) FROM agent_skills WHERE name=?1",
                [&candidate_name],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0
        {
            candidate_name = format!(
                "{} · {}",
                name.chars().take(60).collect::<String>(),
                knowledge_id
            );
        }
        connection.execute("INSERT INTO agent_skills(name,description,instructions,builtin,enabled) VALUES(?1,?2,?3,0,1)", params![candidate_name,summary,merged]).map_err(|error| error.to_string())?;
        connection.last_insert_rowid()
    };
    connection.execute("UPDATE agent_knowledge_entries SET skill_id=?1,updated_at=datetime('now','localtime') WHERE id=?2", params![skill_id,knowledge_id]).map_err(|error| error.to_string())?;
    Ok(skill_id)
}

#[tauri::command]
pub fn refine_agent_instruction_with_knowledge(
    state: State<AppState>,
    skill_id: i64,
) -> Result<i64, String> {
    let connection = db::open(&state.db_path)?;
    let (builtin, name, description, instructions): (i64, String, String, String) = connection
        .query_row(
            "SELECT builtin,name,description,instructions FROM agent_skills WHERE id=?1",
            [skill_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|_| "目标 Skill 不存在".to_string())?;
    let candidate = serde_json::json!({
        "title": format!("{} 最新知识精炼", name),
        "summary": description,
        "qualityGate": {"disposition": "reusable_candidate"},
        "skillPatch": {"addSections": []}
    });
    let catalog = agent_learning_catalog(&connection, Some(skill_id))?;
    let patch = refine_learning_patch_for_apply(&candidate, &instructions, &catalog, |prompt| {
        super::learning_model::complete_on(&connection, prompt)
    })?;
    let refined = apply_skill_patch(&instructions, &patch);
    if refined.trim().is_empty() {
        return Err("最新知识精炼没有返回可保存的内容".into());
    }
    if builtin != 0 {
        let clone_name = format!("{} · 最新知识增强", name)
            .chars()
            .take(80)
            .collect::<String>();
        connection.execute("INSERT INTO agent_skills(name,description,instructions,builtin,enabled) VALUES(?1,?2,?3,0,1)", params![clone_name,description,refined]).map_err(|error| error.to_string())?;
        return Ok(connection.last_insert_rowid());
    }
    connection.execute("UPDATE agent_skills SET instructions=?1,updated_at=datetime('now','localtime') WHERE id=?2", params![refined,skill_id]).map_err(|error| error.to_string())?;
    Ok(skill_id)
}

#[tauri::command]
pub fn export_agent_knowledge(state: State<AppState>) -> Result<String, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement = connection.prepare("SELECT title,summary,patterns_json,skill_instructions,source_hash FROM agent_knowledge_entries ORDER BY id").map_err(|error|error.to_string())?;
    let entries = statement.query_map([], |row| Ok(serde_json::json!({"title":row.get::<_,String>(0)?,"summary":row.get::<_,String>(1)?,"patterns":json(row.get::<_,String>(2)?),"skillInstructions":row.get::<_,String>(3)?,"sourceHash":row.get::<_,String>(4)?}))).map_err(|error|error.to_string())?.flatten().collect::<Vec<_>>();
    let path = portable_export_path(&state.export_dir, "agent-knowledge")?;
    let payload = serde_json::json!({"schemaVersion":1,"kind":"oviraptor-agent-knowledge","exportedAt":chrono::Utc::now().to_rfc3339(),"entries":entries});
    fs::write(
        &path,
        serde_json::to_vec_pretty(&payload).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn import_agent_knowledge(state: State<AppState>, path: String) -> Result<i64, String> {
    import_agent_knowledge_path(&state.db_path, Path::new(&path))
}

pub(crate) fn import_agent_knowledge_path(db_path: &Path, path: &Path) -> Result<i64, String> {
    let payload: JsonValue =
        serde_json::from_slice(&fs::read(path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    if payload.get("kind").and_then(JsonValue::as_str) != Some("oviraptor-agent-knowledge") {
        return Err("不是 Oviraptor Agent 知识库导出文件".into());
    }
    let connection = db::open(db_path)?;
    let mut imported = 0;
    for entry in payload
        .get("entries")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
    {
        let title = entry
            .get("title")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim();
        let instructions = entry
            .get("skillInstructions")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim();
        let source_hash = entry
            .get("sourceHash")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim();
        if title.is_empty() || instructions.is_empty() || source_hash.is_empty() {
            continue;
        }
        let scan_id = format!(
            "imported-{}",
            source_hash.chars().take(32).collect::<String>()
        );
        connection.execute("INSERT INTO agent_knowledge_entries(scan_id,title,summary,patterns_json,skill_instructions,source_hash) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(scan_id) DO UPDATE SET title=excluded.title,summary=excluded.summary,patterns_json=excluded.patterns_json,skill_instructions=excluded.skill_instructions,source_hash=excluded.source_hash,updated_at=datetime('now','localtime')",params![scan_id,title,entry.get("summary").and_then(JsonValue::as_str).unwrap_or(""),entry.get("patterns").cloned().unwrap_or_default().to_string(),instructions,source_hash]).map_err(|error|error.to_string())?;
        imported += 1;
    }
    Ok(imported)
}

/// A producer can read back its persisted source card without importing SQL
/// column ordering or broad catalog command authority into Scanning.
pub(crate) fn knowledge_for_source(
    connection: &rusqlite::Connection,
    source: &str,
) -> rusqlite::Result<AgentKnowledgeEntry> {
    connection.query_row(
        &format!("SELECT {KNOWLEDGE_COLUMNS} FROM agent_knowledge_entries WHERE scan_id=?1"),
        [source],
        knowledge_row,
    )
}
pub(crate) fn candidate_for_source(
    connection: &rusqlite::Connection,
    source: &str,
    hash: &str,
) -> rusqlite::Result<AgentLearningCandidate> {
    connection.query_row(&format!("SELECT {LEARNING_CANDIDATE_COLUMNS} FROM agent_learning_candidates WHERE scan_id=?1 AND source_hash=?2"),params![source,hash],learning_candidate_row)
}
