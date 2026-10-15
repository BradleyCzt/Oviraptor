//! Bounded global refinement context, excluding project-owned task knowledge.

use serde_json::Value as JsonValue;

pub(crate) fn agent_learning_catalog(
    connection: &rusqlite::Connection,
    exclude_skill_id: Option<i64>,
) -> Result<String, String> {
    let mut sections = Vec::new();
    let mut skills = connection
        .prepare("SELECT id,name,description,instructions,builtin FROM agent_skills WHERE enabled=1 AND (?1 IS NULL OR id<>?1) ORDER BY builtin DESC,updated_at DESC,id DESC LIMIT 24")
        .map_err(|error| error.to_string())?;
    let rows = skills
        .query_map([exclude_skill_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    for row in rows.flatten() {
        sections.push(format!(
            "Skill #{} [{}] {}\n描述：{}\n指令：{}",
            row.0,
            if row.4 != 0 { "内置" } else { "自定义" },
            row.1,
            row.2.chars().take(700).collect::<String>(),
            row.3.chars().take(2600).collect::<String>()
        ));
    }
    let mut knowledge = connection
        // This is a global refinement catalog, not a task-scoped retrieval.
        // Per-project task cards must not leak into another project's prompt.
        .prepare("SELECT id,title,summary,patterns_json,skill_instructions FROM agent_knowledge_entries WHERE project_id IS NULL ORDER BY updated_at DESC,id DESC LIMIT 96")
        .map_err(|error| error.to_string())?;
    let rows = knowledge
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut knowledge_added = 0;
    for row in rows.flatten() {
        let patterns = serde_json::from_str::<JsonValue>(&row.3)
            .unwrap_or_else(|_| JsonValue::Object(Default::default()));
        let quality = patterns
            .get("qualityScore")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        if quality < 70 {
            continue;
        }
        let kind = patterns
            .get("knowledgeKind")
            .and_then(JsonValue::as_str)
            .unwrap_or("");
        let distinct_scans = patterns
            .pointer("/support/distinctScans")
            .and_then(JsonValue::as_i64)
            .unwrap_or_else(|| if kind == "aggregate" { 2 } else { 1 });
        if !matches!(kind, "aggregate" | "external_source")
            || distinct_scans < 2 && kind == "aggregate"
        {
            // A manually reviewed one-off can still be converted explicitly,
            // but task-derived cards never enter this cross-project catalog.
            continue;
        }
        sections.push(format!(
            "知识 #{} {}（质量 {}）\n摘要：{}\n方法：{}",
            row.0,
            row.1,
            quality,
            row.2.chars().take(700).collect::<String>(),
            row.4.chars().take(1800).collect::<String>()
        ));
        knowledge_added += 1;
        if knowledge_added >= 16 {
            break;
        }
    }
    let mut catalog = sections.join("\n\n");
    if catalog.chars().count() > 42_000 {
        catalog = catalog.chars().take(42_000).collect();
    }
    Ok(if catalog.is_empty() {
        "（暂无其它 Skill 或高质量知识）".into()
    } else {
        catalog
    })
}

#[cfg(test)]
#[test]
fn global_learning_catalog_excludes_task_data_and_bounds_context() {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection.execute_batch(
        "CREATE TABLE agent_skills(id INTEGER PRIMARY KEY,name TEXT,description TEXT,instructions TEXT,builtin INTEGER,enabled INTEGER,updated_at TEXT);
         CREATE TABLE agent_knowledge_entries(id INTEGER PRIMARY KEY,project_id INTEGER,title TEXT,summary TEXT,patterns_json TEXT,skill_instructions TEXT,updated_at TEXT);",
    ).unwrap();
    connection
        .execute(
            "INSERT INTO agent_skills VALUES(1,'active','','safe',1,1,'2026-01-01')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO agent_skills VALUES(2,'disabled','','secret',0,0,'2026-01-02')",
            [],
        )
        .unwrap();
    let insert = |id: i64, project: Option<i64>, kind: &str, support: i64| {
        let patterns = serde_json::json!({
            "knowledgeKind":kind,"qualityScore":90,
            "support":{"distinctScans":support}
        });
        let updated = if id == 4 || id == 5 {
            "2026-01-02"
        } else {
            "2026-01-01"
        };
        connection
            .execute(
                "INSERT INTO agent_knowledge_entries VALUES(?1,?2,?3,'summary',?4,'method',?5)",
                rusqlite::params![
                    id,
                    project,
                    format!("entry-{id}"),
                    patterns.to_string(),
                    updated
                ],
            )
            .unwrap();
    };
    insert(1, Some(7), "aggregate", 4);
    insert(2, None, "task_candidate", 2);
    insert(3, None, "aggregate", 1);
    insert(4, None, "aggregate", 2);
    insert(5, None, "external_source", 1);
    for id in 6..30 {
        insert(id, None, "aggregate", 2);
    }
    let catalog = agent_learning_catalog(&connection, None).unwrap();
    assert!(catalog.contains("active"));
    assert!(!catalog.contains("disabled"));
    assert!(!catalog.contains("entry-1（"));
    assert!(!catalog.contains("entry-2（"));
    assert!(!catalog.contains("entry-3（"));
    assert!(catalog.contains("entry-4（"));
    assert!(catalog.contains("entry-5（"));
    assert_eq!(catalog.matches("知识 #").count(), 16);
    assert!(catalog.chars().count() <= 42_000);
}
