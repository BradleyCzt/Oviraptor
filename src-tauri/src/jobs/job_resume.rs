use super::*;

pub(super) struct PreparedResume {
    pub project_id: i64,
    pub pipeline: String,
    pub settings: Value,
    pub run_dir: PathBuf,
}

pub(super) fn freeze_content_rules(
    connection: &rusqlite::Connection,
    settings: &mut Value,
    new_task: bool,
) -> Result<(), String> {
    let object = settings.as_object_mut().ok_or("原任务配置快照不是对象")?;
    let present = ["_assetContentKeywords", "_assetTitleRules"]
        .iter()
        .filter(|key| object.contains_key(**key))
        .count();
    if !new_task && present == 1 {
        return Err("原任务内容规则快照不完整；请新建任务".into());
    }
    if new_task || present == 0 {
        let mut statement = connection
            .prepare(
                "SELECT keyword,normalized_keyword FROM content_rules WHERE enabled=1 ORDER BY id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;
        let rows = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        object.insert(
            "_assetContentKeywords".into(),
            serde_json::json!(rows.iter().map(|(keyword, _)| keyword).collect::<Vec<_>>()),
        );
        object.insert(
            "_assetTitleRules".into(),
            serde_json::json!(rows
                .iter()
                .map(|(_, normalized)| normalized)
                .collect::<Vec<_>>()),
        );
        if !new_task {
            object.insert("_assetLegacyRulesAdopted".into(), Value::Bool(true));
        }
    }
    for key in ["_assetContentKeywords", "_assetTitleRules"] {
        if !object
            .get(key)
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().all(Value::is_string))
        {
            return Err("原任务内容规则快照损坏；请新建任务".into());
        }
    }
    Ok(())
}

pub(super) fn import_title_rules(
    connection: &rusqlite::Connection,
    run_id: i64,
) -> Result<Vec<String>, String> {
    let snapshot: Option<String> = connection
        .query_row(
            "SELECT config_snapshot FROM runs WHERE id=?1",
            [run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some(snapshot) = snapshot {
        let value: Value = serde_json::from_str(&snapshot).map_err(|_| "原任务配置快照损坏")?;
        if let Some(items) = value.get("_assetTitleRules") {
            let items = items.as_array().ok_or("原任务内容规则快照损坏")?;
            return items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| "原任务内容规则快照损坏".to_string())
                })
                .collect();
        }
    }
    let mut statement = connection
        .prepare("SELECT normalized_keyword FROM content_rules WHERE enabled=1 ORDER BY id")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

pub(super) fn require_idle_project(
    connection: &rusqlite::Connection,
    project_id: i64,
    except_run: Option<i64>,
) -> Result<(), String> {
    let busy: Option<i64> = connection.query_row(
        "SELECT id FROM runs WHERE project_id=?1 AND status IN ('queued','running','cancel_requested') AND (?2 IS NULL OR id<>?2) LIMIT 1",
        params![project_id, except_run], |row| row.get(0),
    ).optional().map_err(|error| error.to_string())?;
    if let Some(run) = busy {
        return Err(format!("项目已有运行中的任务 #{run}"));
    }
    Ok(())
}

pub(super) fn validate_seeds(path: &Path) -> Result<(), String> {
    let mut reader = csv::Reader::from_path(path)
        .map_err(|error| format!("原任务种子无法读取：{error}；请新建任务"))?;
    let headers = reader.headers().map_err(|error| error.to_string())?.clone();
    let inputs = [
        "names",
        "aliases",
        "domains",
        "icps",
        "ip_ranges",
        "asn_orgs",
        "keywords",
        "icp",
        "ips",
        "asns",
    ];
    if !headers.iter().any(|name| name == "names") || !headers.iter().any(|name| name == "domains")
    {
        return Err("原任务种子格式不完整；请新建任务".into());
    }
    let mut present = false;
    for row in reader.records() {
        let row = row.map_err(|error| format!("原任务种子损坏：{error}；请新建任务"))?;
        present |= headers.iter().enumerate().any(|(index, name)| {
            inputs.contains(&name) && !row.get(index).unwrap_or_default().trim().is_empty()
        });
    }
    if !present {
        return Err("原任务种子为空；请新建任务".into());
    }
    Ok(())
}

pub(super) fn prepare_resume(
    connection: &mut rusqlite::Connection,
    run_id: i64,
    app_data_dir: &Path,
) -> Result<PreparedResume, String> {
    let tx = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let (project_id, profile_id, pipeline, output_dir, status, snapshot, project_status):
        (i64,i64,String,String,String,String,String) = tx.query_row(
        "SELECT r.project_id,COALESCE(r.profile_id,0),r.pipeline,r.output_dir,r.status,r.config_snapshot,p.status FROM runs r JOIN projects p ON p.id=r.project_id WHERE r.id=?1",
        [run_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?)),
    ).map_err(|_| "任务不存在".to_string())?;
    if project_status != "active" {
        return Err("项目已归档；恢复工作空间后才能继续资产任务".into());
    }
    if !matches!(pipeline.as_str(), "full" | "collect" | "reprobe") {
        return Err("该任务类型不支持原地恢复".into());
    }
    if !matches!(status.as_str(), "interrupted" | "cancelled" | "failed") {
        return Err("该任务当前不能继续".into());
    }
    require_idle_project(&tx, project_id, Some(run_id))?;
    let run_dir = PathBuf::from(output_dir);
    let checked = run_dir
        .canonicalize()
        .map_err(|_| "原任务目录不存在；请新建任务")?;
    let jobs_root = app_data_dir
        .join("jobs")
        .canonicalize()
        .map_err(|_| "原任务目录不存在")?;
    if checked.parent() != Some(jobs_root.as_path())
        || fs::symlink_metadata(&run_dir)
            .map_err(|error| error.to_string())?
            .file_type()
            .is_symlink()
    {
        return Err("原任务目录不属于当前应用；拒绝恢复".into());
    }
    if pipeline != "reprobe" {
        validate_seeds(&run_dir.join("seeds.csv"))?;
    }
    let mut settings: Value = serde_json::from_str(&snapshot).map_err(|_| "原任务配置快照损坏")?;
    freeze_content_rules(&tx, &mut settings, false)?;
    let persisted_snapshot = settings.to_string();
    let object = settings.as_object_mut().ok_or("原任务配置快照格式错误")?;
    let current_text: String = tx
        .query_row(
            "SELECT settings_json FROM config_profiles WHERE id=?1",
            [profile_id],
            |row| row.get(0),
        )
        .map_err(|_| "原配置方案不存在；请新建任务")?;
    let current: Value = serde_json::from_str(&current_text).map_err(|_| "当前配置方案损坏")?;
    // Runtime credentials and transport repair may change. Collection contents,
    // probe policy and seeds remain the original run's frozen inputs.
    object.insert(
        "fofaKey".into(),
        Value::String(if pipeline == "reprobe" {
            String::new()
        } else {
            native_config::fofa_key(&current)?
        }),
    );
    object.insert("hackerOneToken".into(), Value::String(String::new()));
    for key in ["proxyUrl", "collectionTimeout", "interval"] {
        if let Some(value) = current.get(key) {
            object.insert(key.into(), value.clone());
        }
    }
    let changed = tx.execute("UPDATE runs SET status='queued',stage=?1,error='',finished_at=NULL,config_snapshot=?3 WHERE id=?2 AND status IN ('failed','cancelled','interrupted')",
        params![if pipeline=="reprobe" {"reprobe"} else {"collect"},run_id,persisted_snapshot]).map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("该任务已由其他进程恢复".into());
    }
    tx.commit().map_err(|error| error.to_string())?;
    Ok(PreparedResume {
        project_id,
        pipeline,
        settings,
        run_dir,
    })
}

#[cfg(test)]
#[path = "job_resume_tests.rs"]
mod tests;
