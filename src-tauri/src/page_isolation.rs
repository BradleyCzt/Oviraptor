//! Project-owned page exclusions. Technical observations and physical assets
//! stay intact; effects retain the exact pre-isolation project state.
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::{fs, io::Write, path::Path};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuleInput {
    pub project_id: i64,
    pub source_asset_id: i64,
    pub kind: String,
    pub value: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApplyInput {
    pub rule: RuleInput,
    pub preview_token: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Rule {
    pub id: i64,
    pub project_id: i64,
    pub kind: String,
    pub value: String,
    pub enabled: bool,
    pub example_title: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Candidate {
    asset_id: i64,
    title: String,
    link: String,
    body_hash: String,
    full_body: bool,
    decision: String,
    note: String,
    is_deleted: bool,
    deleted_at: Option<String>,
    entry_opened_at: Option<String>,
    managed: bool,
}
impl Candidate {
    fn eligible(&self) -> bool {
        let automatic = self.note.starts_with("系统自动");
        !self.is_deleted
            && !self.managed
            && (self.note.is_empty() || automatic)
            && (matches!(
                self.decision.as_str(),
                "pending" | "uncertain" | "not_applicable" | ""
            ) || (self.decision == "rejected" && automatic))
    }
    fn matches(&self, kind: &str, value: &str) -> bool {
        match kind {
            "title_contains" => self.title.to_lowercase().contains(value),
            "body_sha256" => self.full_body && self.body_hash.eq_ignore_ascii_case(value),
            _ => false,
        }
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Preview {
    pub rule: RuleInput,
    pub matched: usize,
    pub eligible: usize,
    pub protected: usize,
    pub already_isolated: usize,
    pub sample: Vec<serde_json::Value>,
    pub preview_token: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApplyResult {
    pub rule_id: i64,
    pub isolated: usize,
    pub protected: usize,
    pub backup_path: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DisableResult {
    pub restored: usize,
    pub retained: usize,
    pub backup_path: String,
}

fn project_on(c: &Connection, project: i64, writable: bool) -> Result<(), String> {
    let status: Option<String> = c
        .query_row("SELECT status FROM projects WHERE id=?1", [project], |r| {
            r.get(0)
        })
        .optional()
        .map_err(|e| e.to_string())?;
    match status.as_deref() {
        Some("active") => Ok(()),
        Some(_) if !writable => Ok(()),
        Some(_) => Err("已归档项目不能修改页面规则".into()),
        None => Err("项目不存在".into()),
    }
}
fn candidates_on(
    c: &Connection,
    project: i64,
    asset: Option<i64>,
) -> Result<Vec<Candidate>, String> {
    // Project-specific manual decisions, authorization and investigations take
    // precedence. Viewing an entry is retained, not an automatic veto.
    let mut s = c.prepare(r#"SELECT a.id,a.title,a.link,
      COALESCE(json_extract(CASE WHEN json_valid(a.extra_json) THEN a.extra_json ELSE '{}' END,'$.body_sha256'),''),
      CASE WHEN json_valid(a.extra_json)
        AND json_extract(a.extra_json,'$.probe_body_complete')='true'
        AND json_extract(a.extra_json,'$.probe_body_truncated')='false'
        AND COALESCE(json_extract(a.extra_json,'$.probe_error'),'')=''
        AND COALESCE(json_extract(a.extra_json,'$.probe_content_range'),'')=''
        AND lower(COALESCE(json_extract(a.extra_json,'$.probe_content_encoding'),'')) IN ('','identity')
        AND CAST(json_extract(a.extra_json,'$.probe_body_bytes') AS TEXT) NOT GLOB '*[^0-9]*'
        AND CAST(json_extract(a.extra_json,'$.probe_body_limit') AS TEXT) NOT GLOB '*[^0-9]*'
        AND CAST(json_extract(a.extra_json,'$.probe_body_bytes') AS INTEGER)>0
        AND CAST(json_extract(a.extra_json,'$.probe_body_bytes') AS INTEGER)<=CAST(json_extract(a.extra_json,'$.probe_body_limit') AS INTEGER)
        AND a.status_code=json_extract(a.extra_json,'$.probe_status_code')
        AND CAST(a.status_code AS INTEGER) BETWEEN 100 AND 599 AND a.status_code<>'206'
        AND a.probe_outcome IN ('web_alive','web_restricted','web_abnormal','browser_render_required','virtual_host_required','blocked_content')
        THEN 1 ELSE 0 END,
      pa.decision,pa.note,pa.is_deleted,pa.deleted_at,pa.entry_opened_at,
      EXISTS(SELECT 1 FROM asset_events e WHERE e.project_id=pa.project_id AND e.asset_id=a.id AND e.event_type IN ('decision','archived','restored'))
      OR EXISTS(SELECT 1 FROM asset_ownership_decisions o WHERE o.project_id=pa.project_id AND o.asset_id=a.id AND (o.source<>'auto' OR o.exposure_eligible=1))
      OR EXISTS(SELECT 1 FROM sentinel_targets st WHERE st.project_id=pa.project_id AND (st.asset_id=a.id OR (st.asset_id IS NULL AND st.url=COALESCE(NULLIF(a.link,''),a.host))))
      FROM project_assets pa JOIN assets a ON a.id=pa.asset_id
      WHERE pa.project_id=?1 AND (?2 IS NULL OR a.id=?2) ORDER BY a.id"#).map_err(|e| e.to_string())?;
    let rows = s
        .query_map(params![project, asset], |r| {
            Ok(Candidate {
                asset_id: r.get(0)?,
                title: r.get(1)?,
                link: r.get(2)?,
                body_hash: r.get(3)?,
                full_body: r.get(4)?,
                decision: r.get(5)?,
                note: r.get(6)?,
                is_deleted: r.get(7)?,
                deleted_at: r.get(8)?,
                entry_opened_at: r.get(9)?,
                managed: r.get(10)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}
fn canonical_input(c: &Connection, mut input: RuleInput) -> Result<RuleInput, String> {
    project_on(c, input.project_id, true)?;
    let source = candidates_on(c, input.project_id, Some(input.source_asset_id))?
        .into_iter()
        .next()
        .ok_or("示例资产不属于当前项目")?;
    input.value = input.value.trim().to_lowercase();
    match input.kind.as_str() {
        "title_contains" => {
            if !(2..=200).contains(&input.value.chars().count())
                || input.value.chars().any(char::is_control)
            {
                return Err("标题规则需要 2–200 个普通字符".into());
            }
            if !source.matches(&input.kind, &input.value) {
                return Err("规则文字不在示例标题中，请重新选择".into());
            }
        }
        "body_sha256" => {
            // The client never supplies a substitute hash for the selected page.
            if !source.full_body
                || source.body_hash.len() != 64
                || !source.body_hash.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("此资产没有完整正文证明；请先复测，或使用明确的标题规则".into());
            }
            input.value = source.body_hash.to_lowercase();
        }
        _ => return Err("页面规则类型无效".into()),
    }
    Ok(input)
}
fn inventory_on(
    c: &Connection,
    input: RuleInput,
) -> Result<(RuleInput, Vec<Candidate>, String), String> {
    let input = canonical_input(c, input)?;
    let rows = candidates_on(c, input.project_id, None)?
        .into_iter()
        .filter(|r| r.matches(&input.kind, &input.value))
        .collect::<Vec<_>>();
    let data = serde_json::to_vec(&(&input, &rows)).map_err(|e| e.to_string())?;
    let token = format!("{:x}", Sha256::digest(data));
    Ok((input, rows, token))
}
pub(crate) fn preview_on(c: &Connection, input: RuleInput) -> Result<Preview, String> {
    let tx = c.unchecked_transaction().map_err(|e| e.to_string())?;
    let (rule, rows, preview_token) = inventory_on(&tx, input)?;
    let eligible = rows.iter().filter(|r| r.eligible()).count();
    let already_isolated = rows.iter().filter(|r| r.is_deleted).count();
    let sample=rows.iter().take(20).map(|r|serde_json::json!({"assetId":r.asset_id,"title":r.title,"link":r.link,"eligible":r.eligible(),"alreadyIsolated":r.is_deleted})).collect();
    let result = Preview {
        rule,
        matched: rows.len(),
        eligible,
        protected: rows.len() - eligible - already_isolated,
        already_isolated,
        sample,
        preview_token,
    };
    tx.commit().map_err(|e| e.to_string())?;
    Ok(result)
}
fn backup(db_path: &Path, operation: &str, data: &impl Serialize) -> Result<String, String> {
    let dir = db_path
        .parent()
        .ok_or("数据库目录无效")?
        .join("page-rule-backups");
    match fs::symlink_metadata(&dir) {
        Ok(m) if !m.is_dir() || m.file_type().is_symlink() => return Err("规则备份目录无效".into()),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&dir).map_err(|e| e.to_string())?
        }
        Err(e) => return Err(e.to_string()),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    let path = dir.join(format!("{operation}-{}.json", uuid::Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec_pretty(data).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    #[cfg(unix)]
    fs::File::open(&dir)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}
fn isolate_on(c: &Connection, project: i64, row: &Candidate, rule: &Rule) -> Result<bool, String> {
    if !row.eligible() {
        return Ok(false);
    }
    let note = format!("系统自动页面隔离：规则 #{}", rule.id);
    c.execute("INSERT INTO asset_page_rule_effects(project_id,asset_id,rule_id,original_decision,original_note,original_deleted_at,applied_note,active) VALUES(?1,?2,?3,?4,?5,?6,?7,1) ON CONFLICT(project_id,asset_id) DO UPDATE SET rule_id=excluded.rule_id,original_decision=excluded.original_decision,original_note=excluded.original_note,original_deleted_at=excluded.original_deleted_at,applied_note=excluded.applied_note,active=1",params![project,row.asset_id,rule.id,row.decision,row.note,row.deleted_at,note]).map_err(|e|e.to_string())?;
    c.execute("UPDATE project_assets SET decision='rejected',note=?1,is_deleted=1,deleted_at=datetime('now','localtime') WHERE project_id=?2 AND asset_id=?3",params![note,project,row.asset_id]).map_err(|e|e.to_string())?;
    c.execute("INSERT INTO asset_events(project_id,asset_id,event_type,summary) VALUES(?1,?2,'page_rule_isolated',?3)",params![project,row.asset_id,note]).map_err(|e|e.to_string())?;
    Ok(true)
}
pub(crate) fn apply_on(
    c: &mut Connection,
    db_path: &Path,
    input: ApplyInput,
) -> Result<ApplyResult, String> {
    let tx = c
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let (rule, rows, token) = inventory_on(&tx, input.rule)?;
    if token != input.preview_token {
        return Err("命中或人工状态已改变，请重新预览后再应用".into());
    }
    let backup_path = backup(
        db_path,
        "apply",
        &serde_json::json!({"schemaVersion":1,"operation":"apply","rule":rule,"inventory":rows,"previewToken":token}),
    )?;
    tx.execute("INSERT INTO asset_page_rules(project_id,kind,value,source_asset_id) VALUES(?1,?2,?3,?4) ON CONFLICT(project_id,kind,value) DO UPDATE SET enabled=1,source_asset_id=excluded.source_asset_id",params![rule.project_id,rule.kind,rule.value,rule.source_asset_id]).map_err(|e|e.to_string())?;
    let id = tx
        .query_row(
            "SELECT id FROM asset_page_rules WHERE project_id=?1 AND kind=?2 AND value=?3",
            params![rule.project_id, rule.kind, rule.value],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let saved = Rule {
        id,
        project_id: rule.project_id,
        kind: rule.kind,
        value: rule.value,
        enabled: true,
        example_title: rows
            .iter()
            .find(|r| r.asset_id == rule.source_asset_id)
            .map(|r| r.title.clone())
            .unwrap_or_default(),
    };
    let mut isolated = 0;
    for row in &rows {
        isolated += usize::from(isolate_on(&tx, rule.project_id, row, &saved)?);
    }
    let protected = rows
        .iter()
        .filter(|r| !r.is_deleted && !r.eligible())
        .count();
    tx.commit().map_err(|e| e.to_string())?;
    Ok(ApplyResult {
        rule_id: id,
        isolated,
        protected,
        backup_path,
    })
}
pub(crate) fn list_on(c: &Connection, project: i64) -> Result<Vec<Rule>, String> {
    project_on(c, project, false)?;
    let mut s=c.prepare("SELECT r.id,r.project_id,r.kind,r.value,r.enabled,COALESCE(a.title,'') FROM asset_page_rules r LEFT JOIN assets a ON a.id=r.source_asset_id WHERE r.project_id=?1 ORDER BY r.id").map_err(|e|e.to_string())?;
    let rows = s
        .query_map([project], |r| {
            Ok(Rule {
                id: r.get(0)?,
                project_id: r.get(1)?,
                kind: r.get(2)?,
                value: r.get(3)?,
                enabled: r.get(4)?,
                example_title: r.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}
pub(crate) fn enabled_on(c: &Connection, project: i64) -> Result<Vec<Rule>, String> {
    Ok(list_on(c, project)?
        .into_iter()
        .filter(|r| r.enabled)
        .collect())
}
/// Called inside the production CSV import transaction, after technical fields
/// and normal classification are updated. No network calls or nested commits.
pub(crate) fn apply_import_on(
    c: &Connection,
    project: i64,
    asset: i64,
    rules: &[Rule],
) -> Result<(), String> {
    if rules.is_empty() {
        return Ok(());
    }
    if c.is_autocommit() {
        return Err("页面规则入库必须在原资产事务中执行".into());
    }
    let row = candidates_on(c, project, Some(asset))?
        .into_iter()
        .next()
        .ok_or("入库资产不属于当前项目")?;
    if let Some(rule) = rules.iter().find(|rule| {
        rule.enabled && rule.project_id == project && row.matches(&rule.kind, &rule.value)
    }) {
        isolate_on(c, project, &row, rule)?;
    }
    Ok(())
}
pub(crate) fn disable_on(
    c: &mut Connection,
    db_path: &Path,
    project: i64,
    rule_id: i64,
) -> Result<DisableResult, String> {
    let tx = c
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    project_on(&tx, project, true)?;
    if !list_on(&tx, project)?.iter().any(|r| r.id == rule_id) {
        return Err("规则不属于当前项目".into());
    }
    let effects = {
        let mut s=tx.prepare("SELECT asset_id,original_decision,original_note,original_deleted_at,applied_note FROM asset_page_rule_effects WHERE project_id=?1 AND rule_id=?2 AND active=1 ORDER BY asset_id").map_err(|e|e.to_string())?;
        let rows = s
            .query_map(params![project, rule_id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };
    let inventory = candidates_on(&tx, project, None)?;
    let effect_ids = effects.iter().map(|e| e.0).collect::<HashSet<_>>();
    let backup_path = backup(
        db_path,
        "disable",
        &serde_json::json!({"schemaVersion":1,"projectId":project,"ruleId":rule_id,"effects":effects,"inventory":inventory.iter().filter(|r|effect_ids.contains(&r.asset_id)).collect::<Vec<_>>()}),
    )?;
    let inventory = inventory
        .into_iter()
        .map(|r| (r.asset_id, r))
        .collect::<HashMap<_, _>>();
    tx.execute(
        "UPDATE asset_page_rules SET enabled=0 WHERE project_id=?1 AND id=?2",
        params![project, rule_id],
    )
    .map_err(|e| e.to_string())?;
    let remaining = enabled_on(&tx, project)?;
    let mut restored = 0;
    let mut retained = 0;
    for (asset, decision, note, deleted_at, applied_note) in effects {
        let row = inventory.get(&asset).ok_or("规则隔离记录缺少资产")?;
        if !row.is_deleted || row.decision != "rejected" || row.note != applied_note || row.managed
        {
            retained += 1;
            continue;
        }
        if let Some(other) = remaining.iter().find(|r| row.matches(&r.kind, &r.value)) {
            let new_note = format!("系统自动页面隔离：规则 #{}", other.id);
            tx.execute("UPDATE asset_page_rule_effects SET rule_id=?1,applied_note=?2 WHERE project_id=?3 AND asset_id=?4",params![other.id,new_note,project,asset]).map_err(|e|e.to_string())?;
            tx.execute(
                "UPDATE project_assets SET note=?1 WHERE project_id=?2 AND asset_id=?3",
                params![new_note, project, asset],
            )
            .map_err(|e| e.to_string())?;
            retained += 1;
            continue;
        }
        tx.execute("UPDATE project_assets SET decision=?1,note=?2,is_deleted=0,deleted_at=?3 WHERE project_id=?4 AND asset_id=?5",params![decision,note,deleted_at,project,asset]).map_err(|e|e.to_string())?;
        tx.execute(
            "UPDATE asset_page_rule_effects SET active=0 WHERE project_id=?1 AND asset_id=?2",
            params![project, asset],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO asset_events(project_id,asset_id,event_type,summary) VALUES(?1,?2,'page_rule_restored',?3)",params![project,asset,format!("停用页面隔离规则 #{rule_id}，恢复原项目状态")]).map_err(|e|e.to_string())?;
        restored += 1;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(DisableResult {
        restored,
        retained,
        backup_path,
    })
}

pub(crate) fn owned_isolation_sql() -> &'static str {
    "EXISTS(SELECT 1 FROM asset_page_rule_effects pe WHERE pe.project_id=pa.project_id AND pe.asset_id=pa.asset_id AND pe.active=1 AND pa.is_deleted=1 AND pa.decision='rejected' AND pa.note=pe.applied_note)"
}

/// An explicit human restore overrides an enabled rule for this asset. Restore
/// the pre-rule decision/note only while our exact applied state still owns it.
pub(crate) fn restore_manual_on(c: &Connection, project: i64, asset: i64) -> Result<(), String> {
    if c.is_autocommit() {
        return Err("人工恢复必须在原资产事务中执行".into());
    }
    // A later explicit decision/authorization still owns the row when its text
    // happens to equal the automatic note. Match the same protection as disable.
    let managed = candidates_on(c, project, Some(asset))?
        .first()
        .is_some_and(|row| row.managed);
    let original: Option<(String, String)> = if managed {
        None
    } else {
        c.query_row("SELECT e.original_decision,e.original_note FROM asset_page_rule_effects e JOIN project_assets pa ON pa.project_id=e.project_id AND pa.asset_id=e.asset_id WHERE e.project_id=?1 AND e.asset_id=?2 AND e.active=1 AND pa.is_deleted=1 AND pa.decision='rejected' AND pa.note=e.applied_note", params![project, asset], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .map_err(|e| e.to_string())?
    };
    if let Some((decision, note)) = original {
        c.execute(
            "UPDATE project_assets SET decision=?1,note=?2 WHERE project_id=?3 AND asset_id=?4",
            params![decision, note, project, asset],
        )
        .map_err(|e| e.to_string())?;
    }
    c.execute(
        "UPDATE asset_page_rule_effects SET active=0 WHERE project_id=?1 AND asset_id=?2",
        params![project, asset],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
#[path = "page_isolation_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "page_isolation_migration_tests.rs"]
mod migration_tests;
