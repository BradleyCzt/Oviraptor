//! Instruction catalog CRUD and explicit portable/local-folder imports.
//! Package-backed instructions remain protected by skill_packages' provenance
//! triggers; these legacy text import paths never detach package entries.

use super::model::{AgentSkill, AgentSkillInput};
use crate::{db, AppState};
use rusqlite::{params, OptionalExtension};
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tauri::State;

#[tauri::command]
pub fn list_agent_instructions(state: State<AppState>) -> Result<Vec<AgentSkill>, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement = connection.prepare(
        "SELECT id,name,description,instructions,builtin,enabled,created_at,updated_at FROM agent_skills ORDER BY builtin DESC,updated_at DESC,id DESC"
    ).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(AgentSkill {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                instructions: row.get(3)?,
                builtin: row.get::<_, i64>(4)? != 0,
                enabled: row.get::<_, i64>(5)? != 0,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub fn save_agent_instruction(
    state: State<AppState>,
    input: AgentSkillInput,
) -> Result<i64, String> {
    let name = input.name.trim();
    let instructions = input.instructions.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err("技能名称不能为空且不能超过 80 个字符".into());
    }
    if instructions.is_empty() || instructions.chars().count() > 30_000 {
        return Err("技能指令不能为空且不能超过 30000 个字符".into());
    }
    let connection = db::open(&state.db_path)?;
    if let Some(id) = input.id {
        let builtin: i64 = connection
            .query_row(
                "SELECT builtin FROM agent_skills WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .map_err(|_| "技能不存在".to_string())?;
        if builtin != 0 {
            return Err("内置技能不可覆盖；请新建自定义技能".into());
        }
        connection.execute("UPDATE agent_skills SET name=?1,description=?2,instructions=?3,enabled=?4,updated_at=datetime('now','localtime') WHERE id=?5", params![name,input.description.trim(),instructions,input.enabled as i64,id]).map_err(|error| error.to_string())?;
        Ok(id)
    } else {
        connection.execute("INSERT INTO agent_skills(name,description,instructions,builtin,enabled) VALUES(?1,?2,?3,0,?4)", params![name,input.description.trim(),instructions,input.enabled as i64]).map_err(|error| error.to_string())?;
        Ok(connection.last_insert_rowid())
    }
}

#[tauri::command]
pub fn delete_agent_instruction(state: State<AppState>, skill_id: i64) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    let deleted = connection
        .execute(
            "DELETE FROM agent_skills WHERE id=?1 AND builtin=0",
            [skill_id],
        )
        .map_err(|error| error.to_string())?;
    if deleted == 0 {
        return Err("内置技能不可删除，或技能不存在".into());
    }
    Ok(())
}

pub(super) fn portable_export_path(root: &Path, prefix: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    Ok(root.join(format!(
        "{prefix}-{}.json",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    )))
}

#[tauri::command]
pub fn export_agent_instructions(state: State<AppState>) -> Result<String, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement = connection.prepare("SELECT name,description,instructions,enabled FROM agent_skills WHERE builtin=0 ORDER BY name").map_err(|error|error.to_string())?;
    let skills = statement.query_map([], |row| Ok(serde_json::json!({"name":row.get::<_,String>(0)?,"description":row.get::<_,String>(1)?,"instructions":row.get::<_,String>(2)?,"enabled":row.get::<_,i64>(3)?!=0}))).map_err(|error|error.to_string())?.flatten().collect::<Vec<_>>();
    let path = portable_export_path(&state.export_dir, "agent-skills")?;
    let payload = serde_json::json!({"schemaVersion":1,"kind":"oviraptor-agent-skills","exportedAt":chrono::Utc::now().to_rfc3339(),"skills":skills});
    fs::write(
        &path,
        serde_json::to_vec_pretty(&payload).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn import_agent_instructions(state: State<AppState>, path: String) -> Result<i64, String> {
    import_agent_instructions_path(&state.db_path, Path::new(&path))
}

pub(crate) fn import_agent_instructions_path(db_path: &Path, path: &Path) -> Result<i64, String> {
    let payload: JsonValue =
        serde_json::from_slice(&fs::read(path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    if payload.get("kind").and_then(JsonValue::as_str) != Some("oviraptor-agent-skills") {
        return Err("不是 Oviraptor Agent Skill 导出文件".into());
    }
    let connection = db::open(db_path)?;
    let mut imported = 0;
    for skill in payload
        .get("skills")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
    {
        let name = skill
            .get("name")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim();
        let instructions = skill
            .get("instructions")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim();
        if name.is_empty()
            || name.chars().count() > 80
            || instructions.is_empty()
            || instructions.chars().count() > 30_000
        {
            continue;
        }
        let builtin = connection
            .query_row(
                "SELECT builtin FROM agent_skills WHERE name=?1",
                [name],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .unwrap_or(None);
        if builtin == Some(1) {
            continue;
        }
        connection.execute("INSERT INTO agent_skills(name,description,instructions,builtin,enabled) VALUES(?1,?2,?3,0,?4) ON CONFLICT(name) DO UPDATE SET description=excluded.description,instructions=excluded.instructions,enabled=excluded.enabled,updated_at=datetime('now','localtime') WHERE agent_skills.builtin=0",params![name,skill.get("description").and_then(JsonValue::as_str).unwrap_or(""),instructions,skill.get("enabled").and_then(JsonValue::as_bool).unwrap_or(true) as i64]).map_err(|error|error.to_string())?;
        imported += 1;
    }
    Ok(imported)
}

pub(crate) fn collect_full_sec_skill_sections(root: &Path) -> Result<(String, usize), String> {
    let mut files = Vec::new();
    fn walk(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
        for entry in fs::read_dir(path)
            .map_err(|error| error.to_string())?
            .flatten()
        {
            let child = entry.path();
            if child.is_dir() {
                let name = child
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("");
                if !name.starts_with('.') {
                    walk(&child, files)?;
                }
            } else if child.is_file() {
                let name = child
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("");
                if !name.starts_with('.') {
                    files.push(child);
                }
            }
        }
        Ok(())
    }
    walk(root, &mut files)?;
    files.sort();
    let mut output = String::from(
        "# sec_skills · 内部完整方法包\n来源：本地 sec_skills；以下文本按文件原样导入，仅作为公司内部授权资产自查的 Skill 内容，不在导入阶段执行任何命令。\n\n",
    );
    for file in &files {
        let text = fs::read_to_string(file).map_err(|error| error.to_string())?;
        let relative = file.strip_prefix(root).unwrap_or(file);
        output.push_str(&format!("## {}\n\n{}\n\n", relative.display(), text.trim()));
    }
    Ok((output.trim().to_string(), files.len()))
}

#[tauri::command]
pub fn import_sec_skill_knowledge(
    state: State<AppState>,
    path: String,
) -> Result<JsonValue, String> {
    let root = PathBuf::from(path.trim());
    if !root.is_dir() {
        return Err("sec_skills 路径不存在或不是目录".into());
    }
    let (instructions, files_scanned) = collect_full_sec_skill_sections(&root)?;
    if files_scanned == 0 || instructions.len() < 200 {
        return Err("目录中没有可导入的文本 Skill 文件；未创建 Skill".into());
    }
    let mut hasher = Sha256::new();
    hasher.update(instructions.as_bytes());
    let source_hash = format!("{:x}", hasher.finalize());
    let connection = db::open(&state.db_path)?;
    let name = "sec_skills · 内部完整方法包";
    let description = format!(
        "从本地 sec_skills 按文件完整导入 {files_scanned} 个文本文件，供公司内部授权资产自查使用。source:{source_hash}"
    );
    upsert_sec_skill_package(&connection, name, &description, &instructions)?;
    let skill_id = connection
        .query_row("SELECT id FROM agent_skills WHERE name=?1", [name], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|error| error.to_string())?;
    Ok(serde_json::json!({
        "skillId": skill_id,
        "name": name,
        "filesScanned": files_scanned,
        "sectionsKept": files_scanned,
        "droppedLines": 0,
        "enabled": true,
        "sourceHash": source_hash,
    }))
}

/// The package row is upserted into `agent_skills`, so the guard that protects a
/// user-visible built-in has to name the same table.
pub(crate) fn upsert_sec_skill_package(
    connection: &rusqlite::Connection,
    name: &str,
    description: &str,
    instructions: &str,
) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO agent_skills(name,description,instructions,builtin,enabled) VALUES(?1,?2,?3,0,1) ON CONFLICT(name) DO UPDATE SET description=excluded.description,instructions=excluded.instructions,enabled=1,updated_at=datetime('now','localtime') WHERE agent_skills.builtin=0",
            params![name, description, instructions],
        )
        .map_err(|error| format!("无法写入方法包技能：{error}"))?;
    Ok(())
}
