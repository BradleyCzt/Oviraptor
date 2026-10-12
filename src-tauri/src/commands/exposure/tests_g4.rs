// G4 — incidents and locations (G4-0), coverage and cross-unit statistics (G4-1), live
// progress (G4-2), keyset pages and exports (G4-3). Network only to localhost fixtures.
// Handling actions are records: no test here (or code path) contacts anyone.

use super::adapters::documents::{self, KIND_DOCUMENT_LINK};
use super::adapters::github::{CredentialLookup, GithubEndpoint};
use super::fixture::{response, routes, seed_scope, Fixture};
use super::incidents::{act, check_schema, page_incidents, page_locations, propose, read_incident};
use super::orchestrator::start_or_reuse;
use super::settings::{save, DocumentUrl, ExposureSettings};
use super::state::ensure_exposure_schema;
use rusqlite::{params, Connection};
use serde_json::{json, Value as JsonValue};
use std::path::{Path, PathBuf};

fn sandbox(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("oviraptor-exposure-g4-{tag}-{}", uuid::Uuid::new_v4()))
}

fn database(root: &Path) -> (Connection, PathBuf) {
    let db_path = crate::db::initialize(&root.join("app")).expect("临时库");
    let connection = crate::db::open(&db_path).expect("连接");
    ensure_exposure_schema(&connection).expect("附加结构必须可用");
    (connection, db_path)
}

fn count(connection: &Connection, table: &str) -> i64 {
    connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0)).unwrap()
}

fn project(connection: &Connection, project_id: i64) {
    connection
        .execute("INSERT OR IGNORE INTO projects(id,name,status) VALUES(?1,?2,'active')", params![project_id, format!("g4-{project_id}")])
        .unwrap();
}

fn run(connection: &Connection, project_id: i64, status: &str) -> i64 {
    project(connection, project_id);
    connection
        .execute("INSERT INTO exposure_runs(project_id,status,stage) VALUES(?1,?2,?2)", params![project_id, status])
        .unwrap();
    connection.last_insert_rowid()
}

#[derive(Clone)]
struct Lead {
    url: String,
    title: String,
    disposition: &'static str,
    category: &'static str,
    text: Option<String>,
    body: String,
    truncated: bool,
    source: &'static str,
    kind: &'static str,
    asset: Option<i64>,
    status_code: i64,
}

fn lead(url: &str, title: &str) -> Lead {
    Lead {
        url: url.to_string(),
        title: title.to_string(),
        disposition: "risk_candidate",
        category: "credential",
        text: None,
        body: String::new(),
        truncated: false,
        source: "passive_web",
        kind: "page",
        asset: None,
        status_code: 200,
    }
}

impl Lead {
    fn text(mut self, hash: &str) -> Self {
        self.text = Some(hash.to_string());
        self
    }
    fn body(mut self, hash: &str) -> Self {
        self.body = hash.to_string();
        self
    }
    fn pending(mut self) -> Self {
        self.disposition = "classification_pending";
        self
    }
}

/// One lead as the production pipeline leaves it: observation, finding, link, disposition.
fn add(connection: &Connection, project_id: i64, run_id: i64, row: &Lead) -> (i64, i64) {
    let mode = super::incidents::location_kind(row.source, row.kind);
    let mut detail = json!({"collectionMode": if mode == "first_party_url" || mode == "document_url" { "http_read" } else { "index" }});
    if let Some(text) = &row.text {
        detail["normalizedTextSha256"] = json!(text);
    }
    connection
        .execute(
            "INSERT INTO exposure_observations(run_id,project_id,source_key,asset_id,kind,url,canonical_url,status_code,content_hash,body_bytes,truncated,evidence_json)
             VALUES(?1,?2,?3,?4,?5,?6,?6,?7,?8,?9,?10,?11)",
            params![
                run_id,
                project_id,
                row.source,
                row.asset,
                row.kind,
                row.url,
                row.status_code,
                row.body,
                if row.body.is_empty() && row.text.is_none() { 0 } else { 120 },
                i64::from(row.truncated),
                json!({"detail": detail}).to_string()
            ],
        )
        .unwrap();
    let observation = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT OR IGNORE INTO exposure_findings(project_id,asset_id,category,title,source_type,source_url,evidence_excerpt,severity,confidence,status,fingerprint)
             VALUES(?1,?2,'secret',?3,?4,?5,'',  'high',70,'new',?6)",
            params![project_id, row.asset, row.title, row.source, row.url, format!("{}|{}", row.url, row.title)],
        )
        .unwrap();
    let finding: i64 = connection
        .query_row("SELECT id FROM exposure_findings WHERE project_id=?1 AND fingerprint=?2", params![project_id, format!("{}|{}", row.url, row.title)], |r| r.get(0))
        .unwrap();
    connection
        .execute(
            "INSERT INTO exposure_finding_observations(finding_id,observation_id,run_id,project_id) VALUES(?1,?2,?3,?4)",
            params![finding, observation, run_id, project_id],
        )
        .unwrap();
    connection
        .execute(
            "INSERT OR REPLACE INTO exposure_finding_dispositions(finding_id,project_id,disposition,info_category,source_channel,reason,policy_version,classification_id,updated_at)
             VALUES(?1,?2,?3,?4,?5,'测试处置',  'test',0,'2026-10-09 00:00:00')",
            params![finding, project_id, row.disposition, row.category, super::classify::source_channel(row.source)],
        )
        .unwrap();
    (finding, observation)
}

fn incident_of(connection: &Connection, finding: i64) -> Option<i64> {
    connection
        .query_row(
            "SELECT l.incident_id FROM exposure_incident_locations l JOIN exposure_incidents i ON i.id=l.incident_id WHERE l.finding_id=?1 AND l.status='active' AND i.status<>'merged'",
            [finding],
            |r| r.get(0),
        )
        .ok()
}

fn revision(connection: &Connection, incident: i64) -> i64 {
    connection.query_row("SELECT revision FROM exposure_incidents WHERE id=?1", [incident], |r| r.get(0)).unwrap()
}

fn status(connection: &Connection, incident: i64) -> String {
    connection.query_row("SELECT status FROM exposure_incidents WHERE id=?1", [incident], |r| r.get(0)).unwrap()
}

fn review(incident: i64, expected: i64, to: &str, op: &str) -> JsonValue {
    json!({"operationId": op, "action": "review", "incidentId": incident, "expectedRevision": expected, "actor": "张三", "reason": "人工复核后的结论", "status": to})
}

fn active_locations(connection: &Connection, incident: i64) -> i64 {
    connection
        .query_row("SELECT COUNT(*) FROM exposure_incident_locations WHERE incident_id=?1 AND status='active'", [incident], |r| r.get(0))
        .unwrap()
}

// ───────────────────────────── G4-0 proposals ─────────────────────────────

#[test]
fn g4_0_identical_full_text_at_two_urls_is_one_candidate_with_two_locations() {
    let root = sandbox("dedup");
    let (connection, _db) = database(&root);
    let pid = 401;
    let r = run(&connection, pid, "completed");
    let (a, _) = add(&connection, pid, r, &lead("https://a.example.test/doc", "文档提及").pending().text("T1"));
    let (b, _) = add(&connection, pid, r, &lead("https://mirror.example.net/doc", "文档提及").pending().text("T1"));
    // Same title, other content: never merged with the pair.
    let (c, _) = add(&connection, pid, r, &lead("https://c.example.test/doc", "文档提及").pending().text("T2"));
    // Same title and both risk, different bodies: two single-lead incidents.
    let (d, _) = add(&connection, pid, r, &lead("https://d.example.test/.env", "疑似密钥").body("B1"));
    let (e, _) = add(&connection, pid, r, &lead("https://e.example.test/.env", "疑似密钥").body("B2"));
    let report = propose(&connection, pid, None).unwrap();
    assert_eq!(report.created, 3, "{report:?}");
    let pair = incident_of(&connection, a).expect("相同全文成为一个事件");
    assert_eq!(incident_of(&connection, b), Some(pair));
    assert_eq!(active_locations(&connection, pair), 2);
    assert_eq!(incident_of(&connection, c), None, "单个待分类文档不是事件");
    assert_ne!(incident_of(&connection, d), incident_of(&connection, e), "同标题不同内容不合并");
    let view = read_incident(&connection, pid, pair).unwrap();
    assert_eq!(view["status"], "candidate");
    assert_eq!(view["legacyFindingStatus"], "new");
    assert_eq!(view["basis"], "identical_normalized_full_text");
    assert_eq!(view["findingIds"].as_array().unwrap().len(), 2);
    for key in ["schemaVersion", "projectId", "runIds", "observationRefs", "findingIds", "stableKey", "revision", "policyVersion", "directoryVersion", "bindingVersion", "collectionModes", "evidenceGrade"] {
        assert!(view.get(key).is_some(), "缺少 {key}");
    }
    // Idempotent: proposing again adds nothing.
    let again = propose(&connection, pid, None).unwrap();
    assert_eq!((again.created, again.attached_locations), (0, 0));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g4_0_truncated_snippets_indexes_and_excluded_leads_never_join_by_content() {
    let root = sandbox("strict");
    let (connection, _db) = database(&root);
    let pid = 402;
    let r = run(&connection, pid, "completed");
    let mut cut = lead("https://a.example.test/x", "疑似密钥").body("SAME").pending();
    cut.truncated = true;
    let (a, _) = add(&connection, pid, r, &cut);
    let (b, _) = add(&connection, pid, r, &lead("https://b.example.test/x", "疑似密钥").body("SAME").pending());
    let mut snippet = lead("https://github.com/o/r/blob/x", "代码命中").body("SAME").pending();
    snippet.source = "github_code";
    snippet.kind = "github_code_hit";
    let (s, _) = add(&connection, pid, r, &snippet);
    let mut excluded = lead("https://gov.example.cn/notice", "采购公告").text("T9");
    excluded.disposition = "excluded_public_procurement";
    let (x, _) = add(&connection, pid, r, &excluded);
    let (y, _) = add(&connection, pid, r, &lead("https://y.example.test/n", "采购公告").pending().text("T9"));
    // Full response bodies from index sources share a hash but are not the same document.
    let mut archive = lead("https://web.archive.org/web/1/a", "归档 A").body("ARCH").pending();
    archive.source = "web_archive";
    archive.kind = "archive_capture";
    let (w1, _) = add(&connection, pid, r, &archive);
    let mut archive2 = lead("https://web.archive.org/web/2/a", "归档 A").body("ARCH").pending();
    archive2.source = "web_archive";
    archive2.kind = "archive_capture";
    let (w2, _) = add(&connection, pid, r, &archive2);
    let mut ct = lead("https://crt.sh/?id=1", "证书").body("ARCH").pending();
    ct.source = "certificate_transparency";
    ct.kind = "certificate";
    let (c1, _) = add(&connection, pid, r, &ct);
    propose(&connection, pid, None).unwrap();
    for finding in [a, b, s, x, y, w1, w2, c1] {
        assert_eq!(incident_of(&connection, finding), None, "截断/摘要/排除不按内容合并 {finding}");
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g4_0_a_new_round_attaches_locations_and_only_flags_a_decided_incident() {
    let root = sandbox("attach");
    let (connection, _db) = database(&root);
    let pid = 403;
    let r1 = run(&connection, pid, "completed");
    let (a, _) = add(&connection, pid, r1, &lead("https://a.example.test/leak.sql", "数据库备份").body("DUMP"));
    propose(&connection, pid, Some(r1)).unwrap();
    let incident = incident_of(&connection, a).unwrap();
    act(&connection, pid, &review(incident, revision(&connection, incident), "verified", "op-attach-001")).unwrap();
    let r2 = run(&connection, pid, "completed");
    let (b, _) = add(&connection, pid, r2, &lead("https://cdn.example.net/leak.sql", "数据库备份").body("DUMP"));
    let report = propose(&connection, pid, Some(r2)).unwrap();
    assert_eq!(report.created, 0);
    assert_eq!(incident_of(&connection, b), Some(incident), "相同内容的新位置挂到已有事件");
    assert_eq!(status(&connection, incident), "verified", "人工结论不被改写");
    let (flag, reason): (i64, String) = connection
        .query_row("SELECT review_needed,review_needed_reason FROM exposure_incidents WHERE id=?1", [incident], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!(flag, 1);
    assert!(reason.contains("新增"), "{reason}");
    // Human review clears the flag.
    act(&connection, pid, &review(incident, revision(&connection, incident), "reviewing", "op-attach-002")).unwrap();
    let flag: i64 = connection.query_row("SELECT review_needed FROM exposure_incidents WHERE id=?1", [incident], |r| r.get(0)).unwrap();
    assert_eq!(flag, 0);
    let _ = std::fs::remove_dir_all(root);
}

// ───────────────────────────── G4-0 human actions ─────────────────────────────

fn one_incident(connection: &Connection, pid: i64) -> (i64, i64, i64) {
    let r = run(connection, pid, "completed");
    let (f, o) = add(connection, pid, r, &lead(&format!("https://p{pid}.example.test/a"), "疑似密钥").body(&format!("X{pid}")));
    propose(connection, pid, Some(r)).unwrap();
    (incident_of(connection, f).unwrap(), f, o)
}

#[test]
fn g4_0_cas_replay_and_operation_conflicts_write_nothing_extra() {
    let root = sandbox("cas");
    let (connection, _db) = database(&root);
    let pid = 404;
    let (incident, _, _) = one_incident(&connection, pid);
    let events = count(&connection, "exposure_incident_events");
    let stale = act(&connection, pid, &review(incident, 99, "reviewing", "op-cas-0001")).unwrap_err();
    assert!(stale.contains("exposure_incident_conflict"), "{stale}");
    assert_eq!(count(&connection, "exposure_incident_events"), events, "冲突零写入");
    let short = act(&connection, pid, &json!({"operationId": "op-cas-0002", "action": "review", "incidentId": incident, "expectedRevision": 1, "actor": "张三", "reason": "短", "status": "reviewing"})).unwrap_err();
    assert!(short.contains("exposure_incident_invalid"), "{short}");
    let unknown = act(&connection, pid, &json!({"operationId": "op-cas-0003", "action": "review", "incidentId": incident, "expectedRevision": 1, "actor": "张三", "reason": "人工复核后的结论", "status": "reviewing", "extra": 1})).unwrap_err();
    assert!(unknown.contains("exposure_incident_invalid"), "未知字段拒绝 {unknown}");
    let first = act(&connection, pid, &review(incident, 1, "reviewing", "op-cas-0004")).unwrap();
    assert_eq!((first["revision"].as_i64(), first["replayed"].as_bool()), (Some(2), Some(false)));
    let replay = act(&connection, pid, &review(incident, 1, "reviewing", "op-cas-0004")).unwrap();
    assert_eq!(replay["replayed"], true);
    assert_eq!(replay["revision"], 2);
    assert_eq!(revision(&connection, incident), 2, "重放不再写");
    let reuse = act(&connection, pid, &review(incident, 2, "verified", "op-cas-0004")).unwrap_err();
    assert!(reuse.contains("exposure_incident_operation_conflict"), "{reuse}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g4_0_two_writers_with_the_same_revision_one_wins() {
    let root = sandbox("race");
    let (connection, db_path) = database(&root);
    let pid = 405;
    let (incident, _, _) = one_incident(&connection, pid);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = ["verified", "dismissed"]
        .iter()
        .enumerate()
        .map(|(index, to)| {
            let barrier = barrier.clone();
            let db_path = db_path.clone();
            let to = to.to_string();
            std::thread::spawn(move || {
                let mine = crate::db::open(&db_path).unwrap();
                barrier.wait();
                act(&mine, pid, &review(incident, 1, &to, &format!("op-race-000{index}")))
            })
        })
        .collect();
    let results: Vec<Result<JsonValue, String>> = handles.into_iter().map(|handle| handle.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|row| row.is_ok()).count(), 1, "{results:?}");
    assert!(results.iter().any(|row| row.as_ref().err().is_some_and(|error| error.contains("exposure_incident_conflict"))), "{results:?}");
    assert_eq!(revision(&connection, incident), 2);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g4_0_merge_and_split_keep_history_and_proposals_respect_them() {
    let root = sandbox("merge");
    let (connection, _db) = database(&root);
    let pid = 406;
    let r = run(&connection, pid, "completed");
    let (a, _) = add(&connection, pid, r, &lead("https://a.example.test/1", "泄漏 A").body("A"));
    let (b1, _) = add(&connection, pid, r, &lead("https://b.example.test/1", "泄漏 B").body("B"));
    let (b2, _) = add(&connection, pid, r, &lead("https://b2.example.test/1", "泄漏 B").body("B"));
    propose(&connection, pid, None).unwrap();
    let ia = incident_of(&connection, a).unwrap();
    let ib = incident_of(&connection, b1).unwrap();
    // A stale target revision must lose, and must not move anything.
    let stale_target = revision(&connection, ib);
    act(&connection, pid, &review(ib, stale_target, "verified", "op-merge-000")).unwrap();
    let stale = act(&connection, pid, &json!({"operationId": "op-merge-stale", "action": "merge", "incidentId": ia, "expectedRevision": revision(&connection, ia), "targetIncidentId": ib, "targetExpectedRevision": stale_target, "actor": "李四", "reason": "用过期的目标版本合并"})).unwrap_err();
    assert!(stale.contains("exposure_incident_conflict"), "{stale}");
    assert_ne!(status(&connection, ia), "merged", "目标版本不符时不能合并");
    let merged = act(&connection, pid, &json!({"operationId": "op-merge-001", "action": "merge", "incidentId": ia, "expectedRevision": revision(&connection, ia), "targetIncidentId": ib, "targetExpectedRevision": revision(&connection, ib), "actor": "李四", "reason": "同一批泄漏的不同副本"})).unwrap();
    assert_eq!(merged["mergedInto"], ib);
    assert_eq!(status(&connection, ia), "merged");
    assert_eq!(active_locations(&connection, ib), 3);
    assert_eq!(active_locations(&connection, ia), 0);
    let late = act(&connection, pid, &review(ia, revision(&connection, ia), "verified", "op-merge-002")).unwrap_err();
    assert!(late.contains("exposure_incident_state"), "{late}");
    // Split b2 back out: at least one location must stay.
    let location: i64 = connection
        .query_row("SELECT id FROM exposure_incident_locations WHERE incident_id=?1 AND finding_id=?2 AND status='active'", params![ib, b2], |r| r.get(0))
        .unwrap();
    let split = act(&connection, pid, &json!({"operationId": "op-split-001", "action": "split", "incidentId": ib, "expectedRevision": revision(&connection, ib), "locationIds": [location], "actor": "李四", "reason": "这一处是另一份文件"})).unwrap();
    let fresh = split["newIncidentId"].as_i64().unwrap();
    assert_eq!(incident_of(&connection, b2), Some(fresh));
    assert_eq!(active_locations(&connection, ib), 2);
    let all: Vec<i64> = {
        let mut statement = connection.prepare("SELECT id FROM exposure_incident_locations WHERE incident_id=?1 AND status='active'").unwrap();
        let rows = statement.query_map([ib], |r| r.get(0)).unwrap().flatten().collect();
        rows
    };
    let refuse = act(&connection, pid, &json!({"operationId": "op-split-002", "action": "split", "incidentId": ib, "expectedRevision": revision(&connection, ib), "locationIds": all, "actor": "李四", "reason": "全部拆走不允许"})).unwrap_err();
    assert!(refuse.contains("至少要保留"), "{refuse}");
    // A later proposal does not pull the split location back.
    propose(&connection, pid, None).unwrap();
    assert_eq!(incident_of(&connection, b2), Some(fresh));
    // The original lead was later excluded, so a new copy of the same content has no live
    // placement of its own: it must follow the merge chain onto the surviving incident.
    connection.execute("UPDATE exposure_finding_dispositions SET disposition='excluded_public_material' WHERE finding_id=?1", [a]).unwrap();
    let (a2, _) = add(&connection, pid, r, &lead("https://cdn.example.net/a2", "泄漏 A").body("A"));
    let again = propose(&connection, pid, None).unwrap();
    assert_eq!(again.created, 0, "合并链上的内容不能另建事件");
    assert_eq!(incident_of(&connection, a2), Some(ib), "新副本沿合并链挂到存活事件");
    let history: i64 = connection.query_row("SELECT COUNT(*) FROM exposure_incident_locations WHERE status IN ('merged','split_out')", [], |r| r.get(0)).unwrap();
    assert_eq!(history, 2, "合并/拆分保留原位置行");
    let _ = std::fs::remove_dir_all(root);
}

fn entity(connection: &Connection, id: i64, kind: &str, name: &str, group: Option<i64>, parent: Option<i64>, region: &str) {
    connection
        .execute(
            "INSERT INTO org_entities(id,group_id,parent_id,entity_kind,legal_name,normalized_name,region_code,directory_version) VALUES(?1,?2,?3,?4,?5,?6,?7,1)",
            params![id, group, parent, kind, name, format!("{name}-{id}"), region],
        )
        .unwrap();
}

fn directory(connection: &Connection) {
    connection.execute("INSERT OR REPLACE INTO org_directory_state(id,version) VALUES(1,5)", []).unwrap();
    entity(connection, 1, "group", "中国移动通信集团有限公司", None, None, "");
    entity(connection, 2, "province_company", "中国移动通信集团浙江有限公司", Some(1), Some(1), "330000");
    entity(connection, 3, "professional_subsidiary", "咪咕文化科技有限公司", Some(1), Some(1), "");
    entity(connection, 4, "city_branch", "咪咕浙江杭州分公司", Some(1), Some(3), "330100");
    entity(connection, 5, "city_branch", "中国移动杭州分公司", Some(1), Some(2), "330100");
    entity(connection, 6, "county_branch", "中国移动余杭分公司", Some(1), Some(5), "330110");
    entity(connection, 9, "province_company", "中国移动通信集团江苏有限公司", Some(1), Some(1), "320000");
}

fn link(connection: &Connection, pid: i64, incident: i64, entity_id: i64, level: &str, evidence: &[i64], op: &str) -> Result<JsonValue, String> {
    act(connection, pid, &json!({"operationId": op, "action": "link_entity", "incidentId": incident, "expectedRevision": revision(connection, incident), "entityId": entity_id, "linkLevel": level, "evidenceObservationIds": evidence, "actor": "王五", "reason": "按页面署名与备案判断"}))
}

#[test]
fn g4_0_unit_links_reference_the_directory_and_confirmed_needs_evidence() {
    let root = sandbox("links");
    let (connection, _db) = database(&root);
    let pid = 407;
    directory(&connection);
    let (incident, _, observation) = one_incident(&connection, pid);
    let (other, _, other_obs) = one_incident(&connection, 4071);
    let _ = other;
    let entities = count(&connection, "org_entities");
    let assets = count(&connection, "assets");
    assert!(link(&connection, pid, incident, 777, "candidate_link", &[], "op-link-001").unwrap_err().contains("exposure_incident_entity_invalid"));
    assert!(link(&connection, pid, incident, 5, "confirmed_link", &[], "op-link-002").unwrap_err().contains("exposure_incident_evidence_required"));
    assert!(link(&connection, pid, incident, 5, "confirmed_link", &[other_obs], "op-link-003").unwrap_err().contains("exposure_incident_evidence_invalid"));
    let ok = link(&connection, pid, incident, 5, "confirmed_link", &[observation], "op-link-004").unwrap();
    assert_eq!(ok["linkLevel"], "confirmed_link");
    let view = read_incident(&connection, pid, incident).unwrap();
    assert_eq!(view["entities"][0]["legalName"], "中国移动杭州分公司");
    assert_eq!(view["entities"][0]["directoryVersion"], 5);
    assert_eq!(view["units"]["confirmed"], 1);
    assert_eq!((count(&connection, "org_entities"), count(&connection, "assets")), (entities, assets), "不新建实体、不改资产");
    // Verified needs a traceable observation: drop it and the verdict is refused, then restore.
    connection.execute("UPDATE exposure_incident_locations SET observation_id=NULL WHERE incident_id=?1", [incident]).unwrap();
    let no_evidence = act(&connection, pid, &review(incident, revision(&connection, incident), "verified", "op-link-005")).unwrap_err();
    assert!(no_evidence.contains("exposure_incident_evidence_required"), "{no_evidence}");
    assert_eq!(status(&connection, incident), "candidate", "没有证据时核实零写入");
    connection.execute("UPDATE exposure_incident_locations SET observation_id=?2 WHERE incident_id=?1", params![incident, observation]).unwrap();
    act(&connection, pid, &review(incident, revision(&connection, incident), "verified", "op-link-006")).unwrap();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g4_0_unit_project_binding_gives_confirmed_links_only_for_confirmed_assets() {
    let root = sandbox("binding");
    let (connection, _db) = database(&root);
    let pid = 408;
    directory(&connection);
    seed_scope(&connection, pid, "https://unit.example.test", true);
    seed_scope(&connection, pid, "https://maybe.example.test", true);
    connection
        .execute("INSERT INTO project_org_bindings(project_id,project_kind,group_entity_id,unit_entity_id,binding_version) VALUES(?1,'unit',1,5,3)", [pid])
        .unwrap();
    let asset = |key: &str| -> i64 { connection.query_row("SELECT id FROM assets WHERE asset_key=?1", [key], |r| r.get(0)).unwrap() };
    let confirmed = asset("https://unit.example.test");
    let maybe = asset("https://maybe.example.test");
    connection.execute("UPDATE asset_ownership_decisions SET status='attributed' WHERE project_id=?1 AND asset_id=?2", params![pid, maybe]).unwrap();
    let r = run(&connection, pid, "completed");
    let mut first = lead("https://unit.example.test/.env", "疑似密钥").body("U1");
    first.asset = Some(confirmed);
    let mut second = lead("https://maybe.example.test/.env", "疑似密钥").body("U2");
    second.asset = Some(maybe);
    let (f1, _) = add(&connection, pid, r, &first);
    let (f2, _) = add(&connection, pid, r, &second);
    propose(&connection, pid, Some(r)).unwrap();
    let level = |finding: i64| -> (String, i64, i64) {
        connection
            .query_row(
                "SELECT link_level,binding_version,directory_version FROM exposure_incident_entities WHERE incident_id=?1 AND active=1",
                [incident_of(&connection, finding).unwrap()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
    };
    assert_eq!(level(f1), ("confirmed_link".to_string(), 3, 5));
    assert_eq!(level(f2).0, "candidate_link");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g4_0_handling_is_recorded_never_sent_and_retests_never_close() {
    let root = sandbox("handling");
    let (connection, _db) = database(&root);
    let watcher = Fixture::start(routes(vec![("/", vec![response(200, &[], "x")])]));
    let pid = 409;
    let (incident, _, _) = one_incident(&connection, pid);
    let early = act(&connection, pid, &json!({"operationId": "op-hand-001", "action": "record_action", "incidentId": incident, "expectedRevision": revision(&connection, incident), "handling": "closed_recorded", "actor": "赵六", "reason": "尚未复核就关闭"})).unwrap_err();
    assert!(early.contains("exposure_incident_state"), "{early}");
    act(&connection, pid, &review(incident, revision(&connection, incident), "verified", "op-hand-002")).unwrap();
    let missing = act(&connection, pid, &json!({"operationId": "op-hand-003", "action": "record_action", "incidentId": incident, "expectedRevision": revision(&connection, incident), "handling": "notification_recorded", "actor": "赵六", "reason": "已通知平台"})).unwrap_err();
    assert!(missing.contains("渠道"), "{missing}");
    let runs = count(&connection, "exposure_runs");
    let items = count(&connection, "exposure_work_items");
    let noted = act(&connection, pid, &json!({"operationId": "op-hand-004", "action": "record_action", "incidentId": incident, "expectedRevision": revision(&connection, incident), "handling": "notification_recorded", "channel": "电话通知省公司安全接口人", "actor": "赵六", "reason": "人工电话通知已完成"})).unwrap();
    assert_eq!(noted["contactedByApplication"], false);
    assert_eq!(noted["handlingState"], "notified");
    act(&connection, pid, &json!({"operationId": "op-hand-005", "action": "record_action", "incidentId": incident, "expectedRevision": revision(&connection, incident), "handling": "takedown_requested_recorded", "channel": "平台工单", "externalReference": "TICKET-1", "actor": "赵六", "reason": "人工在平台提交下架工单"})).unwrap();
    assert_eq!((count(&connection, "exposure_runs"), count(&connection, "exposure_work_items")), (runs, items), "登记不创建采集");
    assert_eq!(watcher.request_count(), 0, "没有任何出站请求");
    // Retest: the later round saw 404 at the location. Recorded as unreachable, never "gone".
    let url: String = connection.query_row("SELECT canonical_url FROM exposure_incident_locations WHERE incident_id=?1", [incident], |r| r.get(0)).unwrap();
    let foreign = run(&connection, 4091, "completed");
    let wrong = act(&connection, pid, &json!({"operationId": "op-hand-006", "action": "record_action", "incidentId": incident, "expectedRevision": revision(&connection, incident), "handling": "retest_linked", "retestRunId": foreign, "actor": "赵六", "reason": "关联复测轮次"})).unwrap_err();
    assert!(wrong.contains("exposure_incident_retest_invalid"), "{wrong}");
    let running = run(&connection, pid, "running");
    let live = act(&connection, pid, &json!({"operationId": "op-hand-006b", "action": "record_action", "incidentId": incident, "expectedRevision": revision(&connection, incident), "handling": "retest_linked", "retestRunId": running, "actor": "赵六", "reason": "关联还在进行的轮次"})).unwrap_err();
    assert!(live.contains("exposure_incident_retest_invalid"), "{live}");
    connection.execute("UPDATE exposure_runs SET status='cancelled' WHERE id=?1", [running]).unwrap();
    let retest = run(&connection, pid, "partial");
    let mut gone = lead(&url, "复测").body("");
    gone.status_code = 404;
    gone.disposition = "classification_pending";
    add(&connection, pid, retest, &gone);
    let linked = act(&connection, pid, &json!({"operationId": "op-hand-007", "action": "record_action", "incidentId": incident, "expectedRevision": revision(&connection, incident), "handling": "retest_linked", "retestRunId": retest, "actor": "赵六", "reason": "关联复测轮次"})).unwrap();
    assert_eq!(linked["retest"]["locations"][0]["outcome"], "unreachable_or_failed");
    assert_eq!(linked["retest"]["autoClosed"], false);
    assert_eq!(status(&connection, incident), "verified", "404 不自动关闭");
    let closed = act(&connection, pid, &json!({"operationId": "op-hand-008", "action": "record_action", "incidentId": incident, "expectedRevision": revision(&connection, incident), "handling": "closed_recorded", "actor": "赵六", "reason": "平台确认已下架，人工关闭"})).unwrap();
    assert_eq!(closed["closureBasis"], "manual_with_retest");
    let blocked = act(&connection, pid, &json!({"operationId": "op-hand-009", "action": "record_action", "incidentId": incident, "expectedRevision": revision(&connection, incident), "handling": "remediation_recorded", "actor": "赵六", "reason": "关闭后不能继续登记"})).unwrap_err();
    assert!(blocked.contains("先重新打开"), "{blocked}");
    act(&connection, pid, &json!({"operationId": "op-hand-010", "action": "record_action", "incidentId": incident, "expectedRevision": revision(&connection, incident), "handling": "reopened", "actor": "赵六", "reason": "复查发现仍可访问"})).unwrap();
    let trail = read_incident(&connection, pid, incident).unwrap();
    assert!(trail["events"].as_array().unwrap().len() >= 7);
    assert_eq!(watcher.request_count(), 0);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g4_0_another_project_reads_and_writes_nothing() {
    let root = sandbox("cross");
    let (connection, _db) = database(&root);
    let (incident, _, _) = one_incident(&connection, 410);
    project(&connection, 411);
    let before = (count(&connection, "exposure_incident_events"), revision(&connection, incident));
    let error = act(&connection, 411, &review(incident, 1, "verified", "op-cross-001")).unwrap_err();
    assert!(error.contains("exposure_incident_not_found"), "{error}");
    assert!(read_incident(&connection, 411, incident).unwrap_err().contains("exposure_incident_not_found"));
    assert!(page_locations(&connection, 411, incident, 10, "").unwrap_err().contains("exposure_incident_not_found"));
    assert_eq!(page_incidents(&connection, 411, "", false, 10, "").unwrap()["total"], 0);
    assert_eq!((count(&connection, "exposure_incident_events"), revision(&connection, incident)), before);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g4_0_an_old_table_missing_columns_is_refused_not_read_as_empty() {
    let root = sandbox("schema");
    let (connection, _db) = database(&root);
    project(&connection, 1);
    check_schema(&connection).unwrap();
    connection.execute_batch("DROP TABLE exposure_incident_events; CREATE TABLE exposure_incident_events(id INTEGER PRIMARY KEY, project_id INTEGER, incident_id INTEGER, operation_id TEXT);").unwrap();
    let error = check_schema(&connection).unwrap_err();
    assert!(error.contains("exposure_incident_schema_mismatch") && error.contains("payload_hash"), "{error}");
    assert!(page_incidents(&connection, 1, "", false, 10, "").unwrap_err().contains("exposure_incident_schema_mismatch"));
    let summary = super::summary::exposure_scope_summary(&connection, 1).unwrap();
    assert!(summary.coverage.is_none() && summary.coverage_error.unwrap().contains("schema_mismatch"), "汇总说明原因，不显示空矩阵");
    let _ = std::fs::remove_dir_all(root);
}

// ───────────────────────────── G4-0 production worker ─────────────────────────────

fn html(body: &str) -> String {
    response(200, &[("Content-Type", "text/html; charset=utf-8")], body)
}

fn entry(url: &str) -> DocumentUrl {
    DocumentUrl { url: documents::canonical_document_url(url).unwrap(), public_material: false, note: String::new() }
}

fn document_project(connection: &Connection, project_id: i64, site: &Fixture, sources: &[&str], urls: Vec<DocumentUrl>) -> JsonValue {
    seed_scope(connection, project_id, &site.origin, true);
    connection
        .execute("UPDATE asset_ownership_profiles SET approved_domains_json=?2 WHERE project_id=?1", params![project_id, r#"["example.test"]"#])
        .unwrap();
    let mut value = ExposureSettings::for_project(project_id);
    value.sources = sources.iter().map(|row| row.to_string()).collect();
    value.allow_private_destinations = true;
    value.timeout_seconds = 5;
    value.document_urls = urls;
    let current: i64 = connection
        .query_row("SELECT COALESCE(MAX(version),0) FROM exposure_settings WHERE project_id=?1", [project_id], |row| row.get(0))
        .unwrap();
    save(connection, &value, current).expect("配置必须能保存");
    super::super::exposure_scan_settings(connection, project_id).expect("配置必须可读")
}

fn fixture_endpoints() -> super::super::SourceEndpoints {
    super::super::SourceEndpoints {
        ct: Vec::new(),
        archive: Vec::new(),
        github: GithubEndpoint { api_origin: "http://127.0.0.1:9".into(), allow_private: true },
        allow_private: true,
        github_credential: CredentialLookup::production(),
        document_any_port: true,
    }
}

fn run_round(connection: &Connection, db_path: &Path, project_id: i64, settings: &JsonValue) -> i64 {
    let started = start_or_reuse(connection, project_id, settings).expect("计划必须能建立");
    super::super::use_test_endpoints_once(fixture_endpoints());
    super::super::execute_exposure_run(db_path.to_path_buf(), started.run_id, project_id, "g4-worker".into(), settings.clone());
    started.run_id
}

#[test]
fn g4_0_the_production_worker_proposes_one_incident_for_the_same_document_at_two_urls() {
    let site = Fixture::start(routes(vec![("/", vec![response(200, &[("Content-Type", "text/html")], "<html><title>首页</title></html>")])]));
    let page = "<html><title>规范</title><body><p>样本公司 内部接口规范 第三章</p></body></html>";
    let docs = Fixture::start(routes(vec![("/a.html", vec![html(page)]), ("/copy/a.html", vec![html(page)]), ("/other.html", vec![html("<html><body><p>样本公司 另一份文件</p></body></html>")])]));
    let root = sandbox("worker");
    let (connection, db_path) = database(&root);
    let pid = 412;
    let urls = vec![entry(&docs.url("/a.html")), entry(&docs.url("/copy/a.html")), entry(&docs.url("/other.html"))];
    let settings = document_project(&connection, pid, &site, &["passive_web", KIND_DOCUMENT_LINK], urls);
    let run_id = run_round(&connection, &db_path, pid, &settings);
    let run_status: String = connection.query_row("SELECT status FROM exposure_runs WHERE id=?1", [run_id], |r| r.get(0)).unwrap();
    assert_eq!(run_status, "completed");
    let page = page_incidents(&connection, pid, "", false, 50, "").unwrap();
    assert_eq!(page["total"], 1, "{page}");
    let item = &page["items"][0];
    assert_eq!(item["activeLocations"], 2);
    assert_eq!(item["basis"], "identical_normalized_full_text");
    assert_eq!(item["evidenceGrade"], "observed_content");
    let events: String = connection
        .query_row("SELECT operation_id FROM exposure_incident_events WHERE incident_id=?1", [item["id"].as_i64().unwrap()], |r| r.get(0))
        .unwrap();
    assert!(events.starts_with(&format!("auto:{run_id}:")), "{events}");
    let summary = super::summary::exposure_scope_summary(&connection, pid).unwrap();
    let coverage = summary.coverage.expect("覆盖矩阵");
    let row = coverage["rows"].as_array().unwrap().iter().find(|row| row["capability"] == "explicit_document_url").unwrap().clone();
    assert_eq!(row["state"], "executed");
    assert!(row["cells"].as_array().unwrap().iter().any(|cell| cell["state"] == "candidate"), "{row}");
    assert_eq!(row["denominator"]["unit"], "URL");
    assert_eq!(row["denominator"]["executed"], 3);
    let live = summary.live_run.expect("实时进度");
    assert_eq!(live["runId"], run_id);
    assert_eq!(live["active"], false);
    assert_eq!(summary.unit_statistics.unwrap()["groupUnique"]["open"], 1);
    let _ = std::fs::remove_dir_all(root);
}

// ───────────────────────────── G4-1 statistics and coverage ─────────────────────────────

#[test]
fn g4_1_multi_unit_incidents_count_once_at_group_and_province_level() {
    let root = sandbox("units");
    let (connection, _db) = database(&root);
    let pid = 413;
    directory(&connection);
    let r = run(&connection, pid, "completed");
    let (x, xo) = add(&connection, pid, r, &lead("https://x.example.test/a", "事件 X").body("X"));
    let (y, yo) = add(&connection, pid, r, &lead("https://y.example.test/a", "事件 Y").body("Y"));
    let (z, _) = add(&connection, pid, r, &lead("https://z.example.test/a", "事件 Z").body("Z"));
    propose(&connection, pid, None).unwrap();
    let (ix, iy, iz) = (incident_of(&connection, x).unwrap(), incident_of(&connection, y).unwrap(), incident_of(&connection, z).unwrap());
    link(&connection, pid, ix, 5, "confirmed_link", &[xo], "op-unit-001").unwrap();
    link(&connection, pid, ix, 4, "candidate_link", &[], "op-unit-002").unwrap();
    link(&connection, pid, iy, 5, "confirmed_link", &[yo], "op-unit-003").unwrap();
    link(&connection, pid, iy, 6, "candidate_link", &[], "op-unit-004").unwrap();
    link(&connection, pid, iy, 9, "conflict", &[], "op-unit-005").unwrap();
    let stats = super::coverage::unit_statistics(&connection, pid, None).unwrap();
    assert_eq!(stats["groupUnique"]["open"], 3, "集团按事件唯一计");
    assert_eq!(stats["sumOfUnitInvolvement"], 5, "单位各计一次，可大于集团数");
    let zhejiang = stats["provinces"].as_array().unwrap().iter().find(|row| row["provinceId"] == 2).unwrap().clone();
    assert_eq!(zhejiang["uniqueIncidents"], 2, "同省多个单位只算一次：{zhejiang}");
    assert_eq!(zhejiang["confirmed"], 2);
    let jiangsu = stats["provinces"].as_array().unwrap().iter().find(|row| row["provinceId"] == 9).unwrap().clone();
    assert_eq!(jiangsu["conflict"], 1);
    let professional = stats["professionalSubsidiaries"].as_array().unwrap();
    assert_eq!(professional.len(), 1);
    assert_eq!(professional[0]["entityId"], 4, "专业公司下的分公司单列，不并入浙江");
    assert_eq!(stats["unknownUnitIncidents"], 1);
    assert_eq!(stats["assets"]["state"], "not_reported", "缺投影显示未报告，不是 0");
    let _ = iz;
    let _ = std::fs::remove_dir_all(root);
}

fn work_item(connection: &Connection, run_id: i64, project_id: i64, source: &str, key: &str, status: &str) {
    connection
        .execute(
            "INSERT INTO exposure_work_items(run_id,project_id,work_key,kind,source_key,status,requests,bytes) VALUES(?1,?2,?3,?4,?4,?5,2,100)",
            params![run_id, project_id, key, source, status],
        )
        .unwrap();
}

#[test]
fn g4_1_coverage_never_turns_a_gap_green_and_keeps_failures_in_the_denominator() {
    let root = sandbox("coverage");
    let (connection, _db) = database(&root);
    let pid = 414;
    project(&connection, pid);
    let mut value = ExposureSettings::for_project(pid);
    value.sources = vec!["passive_web".into(), "certificate_transparency".into(), "web_archive".into()];
    save(&connection, &value, 0).unwrap();
    let r = run(&connection, pid, "partial");
    work_item(&connection, r, pid, "passive_web", "p1", "completed");
    work_item(&connection, r, pid, "passive_web", "p2", "completed");
    work_item(&connection, r, pid, "passive_web", "p3", "failed");
    work_item(&connection, r, pid, "certificate_transparency", "c1", "completed");
    work_item(&connection, r, pid, "web_archive", "w1", "blocked_authorization");
    add(&connection, pid, r, &lead("https://a.example.test/.env", "疑似密钥").body("K"));
    let matrix = super::coverage::coverage_matrix(&connection, pid, None, 7).unwrap();
    let row = |capability: &str| matrix["rows"].as_array().unwrap().iter().find(|row| row["capability"] == capability).unwrap().clone();
    let web = row("first_party_web");
    assert_eq!(web["state"], "partial");
    assert_eq!(web["denominator"]["planned"], 3);
    assert_eq!(web["denominator"]["executed"], 2);
    assert_eq!(web["denominator"]["uncovered"], 1, "失败留在分母");
    assert_eq!(web["denominator"]["outsidePlan"], 7);
    assert_eq!(web["denominator"]["unit"], "资产");
    assert!(web["cells"].as_array().unwrap().iter().all(|cell| cell["tone"] != "green"), "部分执行的行没有绿格");
    let credential = web["cells"].as_array().unwrap().iter().find(|cell| cell["infoCategory"] == "credential").unwrap().clone();
    assert_eq!(credential["state"], "candidate");
    let ct = row("certificate_index");
    assert_eq!(ct["state"], "executed");
    let ct_cells = ct["cells"].as_array().unwrap();
    assert_eq!(ct_cells.iter().find(|cell| cell["infoCategory"] == "technical_network").unwrap()["tone"], "green");
    assert!(ct_cells.iter().filter(|cell| cell["infoCategory"] != "technical_network").all(|cell| cell["state"] == "not_applicable"));
    assert_eq!(row("archive_index")["state"], "pending_authorization");
    assert_eq!(row("repository_metadata")["state"], "unconfigured");
    assert_eq!(row("pdf_docx_ocr")["state"], "not_implemented");
    let import = row("file_import");
    assert_eq!((import["state"].as_str(), import["integration"].as_str()), (Some("unconfigured"), Some("available")));
    assert!(!import["reason"].as_str().unwrap_or("").contains("未注册"));
    let intel = row("public_intel_import");
    assert_eq!(intel["integration"].as_str(), Some("available"));
    assert_ne!(intel["state"].as_str(), Some("not_implemented"));
    assert_eq!(matrix["unitAxis"]["label"], "未报告");
    // Interrupted rounds are their own state, not "no hit".
    let r2 = run(&connection, pid, "interrupted");
    work_item(&connection, r2, pid, "certificate_transparency", "c2", "pending");
    let matrix = super::coverage::coverage_matrix(&connection, pid, None, 0).unwrap();
    let ct = matrix["rows"].as_array().unwrap().iter().find(|row| row["capability"] == "certificate_index").unwrap().clone();
    assert_eq!(ct["state"], "cancelled_interrupted");
    assert!(ct["cells"].as_array().unwrap().iter().all(|cell| cell["tone"] != "green"));
    let _ = std::fs::remove_dir_all(root);
}

// ───────────────────────────── G4-2 live progress ─────────────────────────────

#[test]
fn g4_2_live_progress_reports_items_usage_cancel_and_gaps_without_a_fake_percent() {
    let root = sandbox("live");
    let (connection, _db) = database(&root);
    let pid = 415;
    project(&connection, pid);
    assert!(super::live::live_run(&connection, pid).unwrap().is_none());
    let r = run(&connection, pid, "running");
    connection.execute("UPDATE exposure_runs SET stage='collecting',current_source='passive_web',cancel_requested=1 WHERE id=?1", [r]).unwrap();
    connection
        .execute("INSERT INTO exposure_run_plans(run_id,project_id,budget_requests,budget_bytes,requests_used,bytes_used,planned_items) VALUES(?1,?2,100,1000000,6,300,4)", params![r, pid])
        .unwrap();
    work_item(&connection, r, pid, "passive_web", "p1", "completed");
    work_item(&connection, r, pid, "passive_web", "p2", "in_progress");
    work_item(&connection, r, pid, "web_archive", "w1", "failed");
    work_item(&connection, r, pid, "web_archive", "w2", "pending");
    let live = super::live::live_run(&connection, pid).unwrap().unwrap();
    assert_eq!(live["active"], true);
    assert_eq!(live["stage"], "collecting");
    assert_eq!(live["currentSource"], "passive_web");
    assert_eq!(live["cancelRequested"], true);
    assert_eq!(live["progress"]["done"], 2);
    assert_eq!(live["progress"]["total"], 4);
    assert_eq!(live["progress"]["percent"], 50);
    assert!(live["progress"]["note"].as_str().unwrap().contains("工作项"));
    assert_eq!(live["usage"]["requests"], 6);
    assert_eq!(live["usage"]["budgetRequests"], 100);
    assert_eq!(live["gapsTotal"], 3);
    let archive = live["sources"].as_array().unwrap().iter().find(|row| row["sourceKey"] == "web_archive").unwrap().clone();
    assert_eq!((archive["completed"].as_i64(), archive["total"].as_i64()), (Some(0), Some(2)));
    assert!(!live["lastPersistedAt"].as_str().unwrap().is_empty());
    connection.execute("UPDATE exposure_runs SET status='cancelled',stage='cancelled' WHERE id=?1", [r]).unwrap();
    let ended = super::live::live_run(&connection, pid).unwrap().unwrap();
    assert_eq!(ended["active"], false, "取消后的轮次不再显示为进行中");
    let empty = run(&connection, pid, "queued");
    let live = super::live::live_run(&connection, pid).unwrap().unwrap();
    assert_eq!(live["runId"], empty);
    assert!(live["progress"]["percent"].is_null(), "没有工作项时不给百分比");
    let _ = std::fs::remove_dir_all(root);
}

// ───────────────────────────── G4-3 pages and exports ─────────────────────────────

#[test]
fn g4_3_incident_pages_are_keyset_complete_and_cursor_bound_to_their_filter() {
    let root = sandbox("pages");
    let (connection, _db) = database(&root);
    let pid = 416;
    let r = run(&connection, pid, "completed");
    for index in 0..25 {
        add(&connection, pid, r, &lead(&format!("https://h{index}.example.test/k"), "疑似密钥").body(&format!("S{index}")));
    }
    for index in 0..5 {
        add(&connection, pid, r, &lead(&format!("https://m{index}.example.test/dup"), "副本").body("DUP"));
    }
    propose(&connection, pid, None).unwrap();
    let mut seen = std::collections::BTreeSet::new();
    let mut cursor = String::new();
    let mut pages = 0;
    loop {
        let page = page_incidents(&connection, pid, "", false, 10, &cursor).unwrap();
        assert_eq!(page["total"], 26);
        for item in page["items"].as_array().unwrap() {
            assert!(seen.insert(item["id"].as_i64().unwrap()), "不重复");
        }
        pages += 1;
        cursor = page["nextCursor"].as_str().unwrap().to_string();
        if cursor.is_empty() {
            break;
        }
        let other = page_incidents(&connection, pid, "verified", false, 10, &cursor).unwrap_err();
        assert!(other.contains("exposure_cursor_mismatch"), "{other}");
    }
    assert_eq!((seen.len(), pages), (26, 3));
    assert!(page_incidents(&connection, pid, "", false, 0, "").unwrap_err().contains("exposure_page_size_invalid"));
    assert!(page_incidents(&connection, pid, "bogus", false, 10, "").is_err());
    let dup = page_incidents(&connection, pid, "", false, 200, "").unwrap()["items"].as_array().unwrap().iter().find(|item| item["activeLocations"] == 5).unwrap()["id"].as_i64().unwrap();
    let mut locations = std::collections::BTreeSet::new();
    let mut cursor = String::new();
    loop {
        let page = page_locations(&connection, pid, dup, 2, &cursor).unwrap();
        assert_eq!(page["total"], 5);
        for item in page["items"].as_array().unwrap() {
            locations.insert(item["id"].as_i64().unwrap());
        }
        cursor = page["nextCursor"].as_str().unwrap().to_string();
        if cursor.is_empty() {
            break;
        }
    }
    assert_eq!(locations.len(), 5, "组内分页完整");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g4_3_exports_are_named_escaped_redacted_and_the_ledger_reconciles() {
    let root = sandbox("export");
    let (connection, _db) = database(&root);
    let pid = 417;
    connection.execute("INSERT INTO projects(id,name,status) VALUES(?1,'测试/项目:A','active')", [pid]).unwrap();
    directory(&connection);
    let r = run(&connection, pid, "completed");
    let (x, xo) = add(&connection, pid, r, &lead("https://x.example.test/a", "=HYPERLINK(\"http://evil\") password=SuperSecretValue123").body("X"));
    add(&connection, pid, r, &lead("https://y.example.test/a", "副本").body("Y"));
    add(&connection, pid, r, &lead("https://y2.example.test/a", "副本").body("Y"));
    propose(&connection, pid, None).unwrap();
    link(&connection, pid, incident_of(&connection, x).unwrap(), 5, "confirmed_link", &[xo], "op-exp-001").unwrap();
    let dir = root.join("exports");
    std::fs::create_dir_all(&dir).unwrap();
    let now = chrono::Local::now();
    let (first, rows) = super::super::export_exposure_in(&connection, &dir, pid, "findings", now).unwrap();
    let (second, _) = super::super::export_exposure_in(&connection, &dir, pid, "findings", now).unwrap();
    let name = first.file_name().unwrap().to_string_lossy().to_string();
    assert!(name.starts_with("测试_项目_A_暴露面_"), "{name}");
    let stamp = now.format("%Y%m%d-%H%M%S%z").to_string();
    assert!(name.ends_with(&format!("{stamp}.csv")), "{name} 应带本地时间与时区");
    assert!(second.file_name().unwrap().to_string_lossy().ends_with(&format!("{stamp}_2.csv")), "同一秒导出加后缀，不覆盖");
    assert_eq!(rows, 3);
    let verified = super::export::verify_written(&first).unwrap();
    assert_eq!(verified["format"], "findings_v2");
    assert_eq!(verified["rows"], 3);
    assert_eq!(verified["sha256"].as_str().unwrap().len(), 64);
    let text = std::fs::read_to_string(&first).unwrap();
    assert!(!text.contains("SuperSecretValue123"), "凭据值不进导出");
    assert!(!text.lines().any(|line| line.starts_with('=') || line.contains(",=")), "公式被转义");
    assert!(text.lines().next().unwrap().contains("事件编号"));
    let (ledger, _) = super::super::export_exposure_in(&connection, &dir, pid, "ledger", now).unwrap();
    assert!(ledger.file_name().unwrap().to_string_lossy().contains("_暴露面_事件台账_"));
    let verified = super::export::verify_written(&ledger).unwrap();
    assert_eq!(verified["format"], "ledger_v1");
    let mut reader = csv::Reader::from_path(&ledger).unwrap();
    let mut totals = std::collections::BTreeMap::new();
    for record in reader.records() {
        let record = record.unwrap();
        if &record[0] == "summary" {
            totals.insert(record[1].to_string(), record[10].parse::<i64>().unwrap());
        }
    }
    let kinds = &verified["rowKinds"];
    assert_eq!(totals["incidents_total"], kinds["incident"].as_i64().unwrap());
    assert_eq!(totals["incidents_total"], 2);
    assert_eq!(totals["locations_total"], kinds["location"].as_i64().unwrap());
    assert_eq!(totals["locations_total"], 3);
    assert_eq!(totals["entity_links_total"], kinds["entity_link"].as_i64().unwrap());
    assert_eq!(totals["coverage_rows_total"], kinds["coverage"].as_i64().unwrap());
    assert!(!std::fs::read_to_string(&ledger).unwrap().contains("SuperSecretValue123"));
    assert!(super::super::export_exposure_in(&connection, &dir, pid, "everything", now).unwrap_err().contains("exposure_export_kind_invalid"));
    // A header with the right count but other names is refused.
    let forged = dir.join("forged.csv");
    let mut header = super::export::LEDGER_HEADERS.to_vec();
    header[1] = "伪造";
    std::fs::write(&forged, format!("{}\n", header.join(","))).unwrap();
    assert!(super::export::verify_written(&forged).unwrap_err().contains("exposure_export_shape"));
    let _ = std::fs::remove_dir_all(root);
}

// ───────────── IR-02/IR-11 group ledger reads the ownership projection ─────────────

/// Writes the rows a completed "confirm ownership" group decision leaves behind (the ownership
/// module's own tests drive the real operation; here the exposure side only reads them).
fn ownership_group_decision(connection: &Connection, pid: i64, asset: i64, entity: i64, op: &str, lane: &str) {
    connection
        .execute(
            "INSERT INTO ownership_analysis_runs(project_id,status,scope_version,data_revision,policy_version,directory_version,binding_version,algorithm_version) VALUES(?1,'completed','s','d',1,5,3,'a')",
            [pid],
        )
        .unwrap();
    let run = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO ownership_review_groups(run_id,project_id,group_key,lane,nav_bucket,candidate_entity_id,candidate_source,relation_kind,boundary_kind,support_kind,decision_status,decision_source,authorization_status,exposure_eligible) VALUES(?1,?2,?3,?4,'province',?5,'directory','official_host','exact_host','directory_official_host','auto','auto','unknown',0)",
            params![run, pid, format!("k-{op}"), lane, entity],
        )
        .unwrap();
    let group = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO ownership_operations(operation_id,project_id,run_id,group_id,kind,action,status,payload_hash,expected_action_hash,result_json) VALUES(?1,?2,?3,?4,'group_decision','confirm_ownership','completed','h','h','{}')",
            params![op, pid, run, group],
        )
        .unwrap();
    connection
        .execute("INSERT INTO ownership_operation_items(operation_id,asset_id,outcome) VALUES(?1,?2,'applied')", params![op, asset])
        .unwrap();
    connection
        .execute("UPDATE asset_ownership_decisions SET status='confirmed',source='manual' WHERE project_id=?1 AND asset_id=?2", params![pid, asset])
        .unwrap();
    connection
        .execute(
            "INSERT INTO asset_events(project_id,asset_id,event_type,summary) VALUES(?1,?2,'ownership',?3)",
            params![pid, asset, format!("confirmed: 组确认（复核组 #{group}，快照 #{run}，操作 {op}）")],
        )
        .unwrap();
}

#[test]
fn ir02_group_ledger_incidents_link_units_only_from_the_ownership_projection() {
    let root = sandbox("ir02");
    let (connection, _db) = database(&root);
    let pid = 452;
    directory(&connection);
    for origin in ["https://a.example.test", "https://b.example.test", "https://c.example.test", "https://d.example.test"] {
        seed_scope(&connection, pid, origin, true);
    }
    connection
        .execute("INSERT INTO project_org_bindings(project_id,project_kind,group_entity_id,binding_version) VALUES(?1,'group_ledger',1,3)", [pid])
        .unwrap();
    let asset = |key: &str| -> i64 { connection.query_row("SELECT id FROM assets WHERE asset_key=?1", [key], |r| r.get(0)).unwrap() };
    let (a, b, c, d) = (asset("https://a.example.test"), asset("https://b.example.test"), asset("https://c.example.test"), asset("https://d.example.test"));
    // a: 杭州分公司（浙江省下）；b: 咪咕杭州分公司（专业公司下，名字和地区码都像浙江）；
    // c: 只有人工确认，没有单位依据；d: 冲突组里的候选单位，即使被“确认”也不算。
    ownership_group_decision(&connection, pid, a, 5, "op-ir02-a-0001", "consistent");
    ownership_group_decision(&connection, pid, b, 4, "op-ir02-b-0001", "consistent");
    connection.execute("UPDATE asset_ownership_decisions SET source='manual' WHERE project_id=?1 AND asset_id=?2", params![pid, c]).unwrap();
    ownership_group_decision(&connection, pid, d, 2, "op-ir02-d-0001", "conflict");

    let units = super::units::project_units(&connection, pid).unwrap();
    assert_eq!(units.binding_status, "group_ledger");
    assert_eq!((units.counts.confirmed_group, units.counts.confirmed_unit, units.counts.unassigned), (Some(4), Some(2), 2));
    let row_of = |entity: i64| units.per_unit.iter().find(|row| row.unit.entity_id == entity).cloned();
    let city = row_of(5).expect("杭州分公司");
    assert_eq!(city.province.map(|row| row.entity_id), Some(2), "地市经父级归浙江省公司");
    let migu = row_of(4).expect("咪咕杭州分公司");
    assert_eq!(migu.bucket, "professional_subsidiary");
    assert!(migu.province.is_none(), "专业公司下的分公司不归省");
    assert!(row_of(2).is_none(), "冲突组的候选不算已归属");

    let r = run(&connection, pid, "completed");
    let mut findings = Vec::new();
    for (asset_id, body) in [(a, "A"), (b, "B"), (c, "C"), (d, "D")] {
        let url: String = connection.query_row("SELECT link FROM assets WHERE id=?1", [asset_id], |row| row.get(0)).unwrap();
        let mut row = lead(&format!("{url}/.env"), "疑似密钥").body(body);
        row.asset = Some(asset_id);
        findings.push(add(&connection, pid, r, &row).0);
    }
    propose(&connection, pid, Some(r)).unwrap();
    let links = |finding: i64| -> Vec<(i64, String, String, i64)> {
        let incident = incident_of(&connection, finding).unwrap();
        let mut statement = connection
            .prepare("SELECT entity_id,link_level,origin,active FROM exposure_incident_entities WHERE incident_id=?1 ORDER BY id")
            .unwrap();
        let rows = statement
            .query_map([incident], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        rows
    };
    assert_eq!(links(findings[0]), vec![(5, "confirmed_link".to_string(), "ownership_projection".to_string(), 1)]);
    assert_eq!(links(findings[1]), vec![(4, "confirmed_link".to_string(), "ownership_projection".to_string(), 1)]);
    assert!(links(findings[2]).is_empty(), "没有权威依据：单位未知，不推断");
    assert!(links(findings[3]).is_empty(), "候选不算已归属");
    let stats = super::coverage::unit_statistics(&connection, pid, Some(&units)).unwrap();
    let zhejiang = stats["provinces"].as_array().unwrap().iter().find(|row| row["provinceId"] == 2).cloned().expect("浙江");
    assert_eq!(zhejiang["confirmed"], 1);
    assert_eq!(stats["professionalSubsidiaries"][0]["entityId"], 4);
    assert_eq!(stats["unknownUnitIncidents"], 2);
    assert_eq!(stats["assets"]["confirmedUnit"], 2);
    assert_eq!(stats["assets"]["perAssetEdge"], "ownership_projection");

    // 撤销：组决定之后人工改判 a。统计立即不再计入，再次建议时旧链接失效。
    connection.execute("UPDATE asset_ownership_decisions SET status='excluded' WHERE project_id=?1 AND asset_id=?2", params![pid, a]).unwrap();
    connection.execute("INSERT INTO asset_events(project_id,asset_id,event_type,summary) VALUES(?1,?2,'ownership','excluded: 人工改判')", params![pid, a]).unwrap();
    connection.execute("UPDATE asset_ownership_decisions SET status='confirmed' WHERE project_id=?1 AND asset_id=?2", params![pid, a]).unwrap();
    let after = super::coverage::unit_statistics(&connection, pid, None).unwrap();
    assert!(after["provinces"].as_array().unwrap().iter().all(|row| row["provinceId"] != 2), "撤销后不再计入浙江：{after}");
    assert_eq!(after["unknownUnitIncidents"], 3);
    let r2 = run(&connection, pid, "completed");
    let url: String = connection.query_row("SELECT link FROM assets WHERE id=?1", [a], |row| row.get(0)).unwrap();
    let mut again = lead(&format!("{url}/backup/.env"), "疑似密钥").body("A");
    again.asset = Some(a);
    add(&connection, pid, r2, &again);
    propose(&connection, pid, Some(r2)).unwrap();
    assert_eq!(links(findings[0]), vec![(5, "confirmed_link".to_string(), "ownership_projection".to_string(), 0)], "旧链接失效，不保留为当前");

    // 跨项目：同一资产在另一个集团总账项目里，不能借用本项目的组决定。
    let other = 453;
    seed_scope(&connection, other, "https://b.example.test", true);
    connection
        .execute("INSERT INTO project_org_bindings(project_id,project_kind,group_entity_id,binding_version) VALUES(?1,'group_ledger',1,3)", [other])
        .unwrap();
    connection.execute("UPDATE asset_ownership_decisions SET source='manual' WHERE project_id=?1", [other]).unwrap();
    let foreign = super::units::project_units(&connection, other).unwrap();
    assert_eq!(foreign.counts.confirmed_unit, Some(0), "跨项目零串数");
    let r3 = run(&connection, other, "completed");
    let mut cross = lead("https://b.example.test/.git/config", "疑似密钥").body("X");
    cross.asset = Some(b);
    let (fx, _) = add(&connection, other, r3, &cross);
    propose(&connection, other, Some(r3)).unwrap();
    assert!(links(fx).is_empty(), "另一个项目没有单位链接");
    let _ = std::fs::remove_dir_all(root);
}

// ───────────────────────────── P1 修复回归（2026-10-09） ─────────────────────────────

fn rem_close(incident: i64, expected: i64, op: &str) -> JsonValue {
    json!({"operationId": op, "action": "record_action", "incidentId": incident, "expectedRevision": expected, "actor": "张三", "reason": "复测后人工登记关闭", "handling": "closed_recorded"})
}

fn rem_link_retest(incident: i64, expected: i64, run_id: i64, op: &str) -> JsonValue {
    json!({"operationId": op, "action": "record_action", "incidentId": incident, "expectedRevision": expected, "actor": "张三", "reason": "关联复测轮次", "handling": "retest_linked", "retestRunId": run_id})
}

/// P1-4：失败或没看到的复测不能作为关闭依据；只有已完成轮次里内容确实变化或明确 404/410 才行。
#[test]
fn rem_p1_4_an_incident_whose_latest_retest_failed_cannot_be_closed() {
    let root = sandbox("rem-close");
    let (connection, _db) = database(&root);
    let pid = 471;
    let url = "https://p471.example.test/a";
    let r1 = run(&connection, pid, "completed");
    let (finding, _) = add(&connection, pid, r1, &lead(url, "疑似密钥").body("LEAK471"));
    propose(&connection, pid, Some(r1)).unwrap();
    let incident = incident_of(&connection, finding).unwrap();
    act(&connection, pid, &review(incident, revision(&connection, incident), "verified", "op-rem-close-01")).unwrap();
    let events = count(&connection, "exposure_incident_events");
    // ① 复测轮本身失败（采集中断/失败），即使留下了一条观察。
    let failed = run(&connection, pid, "failed");
    let mut seen = lead(url, "疑似密钥").body("OTHER-BYTES");
    seen.status_code = 200;
    add(&connection, pid, failed, &seen);
    act(&connection, pid, &rem_link_retest(incident, revision(&connection, incident), failed, "op-rem-close-02")).unwrap();
    let refused = act(&connection, pid, &rem_close(incident, revision(&connection, incident), "op-rem-close-03")).unwrap_err();
    assert!(refused.contains("exposure_incident_retest_failed"), "失败的复测轮不能作为关闭依据：{refused}");
    // ② 已完成的复测轮看到 403（取不到），同样不能关闭。
    let forbidden = run(&connection, pid, "completed");
    let mut blocked = lead(url, "疑似密钥").body("FORBIDDEN-PAGE");
    blocked.status_code = 403;
    add(&connection, pid, forbidden, &blocked);
    act(&connection, pid, &rem_link_retest(incident, revision(&connection, incident), forbidden, "op-rem-close-04")).unwrap();
    let before = (count(&connection, "exposure_incident_events"), revision(&connection, incident));
    let refused = act(&connection, pid, &rem_close(incident, revision(&connection, incident), "op-rem-close-05")).unwrap_err();
    assert!(refused.contains("exposure_incident_retest_failed"), "{refused}");
    assert_eq!((count(&connection, "exposure_incident_events"), revision(&connection, incident)), before, "拒绝关闭零写入");
    // ③ 已完成的复测轮仍是同样字节：不能关闭。
    let same = run(&connection, pid, "completed");
    add(&connection, pid, same, &lead(url, "疑似密钥").body("LEAK471"));
    act(&connection, pid, &rem_link_retest(incident, revision(&connection, incident), same, "op-rem-close-06")).unwrap();
    assert!(act(&connection, pid, &rem_close(incident, revision(&connection, incident), "op-rem-close-07")).unwrap_err().contains("exposure_incident_retest_failed"));
    // ④ 已完成的复测轮明确 404：可以由人工关闭，并记为有复测依据。
    let gone = run(&connection, pid, "completed");
    let mut removed = lead(url, "疑似密钥").body("NOT-FOUND");
    removed.status_code = 404;
    add(&connection, pid, gone, &removed);
    act(&connection, pid, &rem_link_retest(incident, revision(&connection, incident), gone, "op-rem-close-08")).unwrap();
    let closed = act(&connection, pid, &rem_close(incident, revision(&connection, incident), "op-rem-close-09")).unwrap();
    assert_eq!(closed["closureBasis"], "manual_with_retest");
    assert!(count(&connection, "exposure_incident_events") > events);
    let _ = std::fs::remove_dir_all(root);
}

/// P1-7：拆分把位置带走时，以这些位置为证据的单位关联跟着去新事件；原事件只保留仍有证据的关联。
#[test]
fn rem_p1_7_split_moves_unit_links_with_their_evidence() {
    let root = sandbox("rem-split");
    let (connection, _db) = database(&root);
    let pid = 472;
    directory(&connection);
    let r = run(&connection, pid, "completed");
    let (fa, oa) = add(&connection, pid, r, &lead("https://a.example.test/leak", "泄漏").body("SPLIT"));
    let (fb, ob) = add(&connection, pid, r, &lead("https://b.example.test/leak", "泄漏").body("SPLIT"));
    propose(&connection, pid, Some(r)).unwrap();
    let incident = incident_of(&connection, fa).unwrap();
    assert_eq!(incident_of(&connection, fb), Some(incident));
    // 杭州分公司的确认关联只以 b 位置为证据；浙江省公司的关联两处都有证据；候选关联没有证据。
    link(&connection, pid, incident, 5, "confirmed_link", &[ob], "op-rem-split-01").unwrap();
    link(&connection, pid, incident, 2, "confirmed_link", &[oa, ob], "op-rem-split-02").unwrap();
    link(&connection, pid, incident, 9, "candidate_link", &[], "op-rem-split-03").unwrap();
    let location_b: i64 = connection
        .query_row("SELECT id FROM exposure_incident_locations WHERE incident_id=?1 AND finding_id=?2 AND status='active'", params![incident, fb], |r| r.get(0))
        .unwrap();
    let split = act(&connection, pid, &json!({"operationId": "op-rem-split-04", "action": "split", "incidentId": incident, "expectedRevision": revision(&connection, incident), "locationIds": [location_b], "actor": "李四", "reason": "b 是另一份文件"})).unwrap();
    let fresh = split["newIncidentId"].as_i64().unwrap();
    let links = |id: i64| -> Vec<(i64, String, String)> {
        let mut statement = connection
            .prepare("SELECT entity_id,link_level,evidence_json FROM exposure_incident_entities WHERE incident_id=?1 AND active=1 ORDER BY entity_id")
            .unwrap();
        let rows = statement.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().flatten().collect();
        rows
    };
    let old = links(incident);
    let new = links(fresh);
    assert_eq!(old.iter().map(|row| row.0).collect::<Vec<_>>(), vec![2, 9], "原事件不再保留只以拆走位置为证据的关联：{old:?}");
    assert_eq!(old[0].2, json!([oa]).to_string(), "原事件的省公司关联只剩仍在本事件的证据");
    assert_eq!(new.iter().map(|row| (row.0, row.1.as_str())).collect::<Vec<_>>(), vec![(2, "confirmed_link"), (5, "confirmed_link")], "新事件带走以拆走位置为证据的关联：{new:?}");
    for (_, _, evidence) in &new {
        assert_eq!(evidence, &json!([ob]).to_string(), "新事件的证据只指向它自己的位置");
    }
    let view = read_incident(&connection, pid, fresh).unwrap();
    assert_eq!(view["units"]["confirmed"], 2, "{}", view["units"]);
    // 新事件的证据真能用：凭它核实。
    act(&connection, pid, &review(fresh, revision(&connection, fresh), "verified", "op-rem-split-05")).unwrap();
    let _ = std::fs::remove_dir_all(root);
}

/// P1-5：人工动作的理由、渠道、外部编号里贴进来的秘密不落库（只存脱敏后的形态）。
#[test]
fn rem_p1_5_secrets_typed_into_incident_actions_never_reach_the_database() {
    let root = sandbox("rem-secret");
    let (connection, _db) = database(&root);
    let pid = 473;
    let (incident, _, _) = one_incident(&connection, pid);
    act(&connection, pid, &review(incident, revision(&connection, incident), "verified", "op-rem-secret-01")).unwrap();
    let secret = "Zq7Hunter2SecretValue99";
    let token = "ghp_RemFixtureToken0123456789abcdefXYZ";
    let reason = format!("通知时对方回了 password={secret}，请轮换");
    let result = act(&connection, pid, &json!({"operationId": "op-rem-secret-02", "action": "record_action", "incidentId": incident, "expectedRevision": revision(&connection, incident), "actor": "张三", "reason": reason, "handling": "notification_recorded", "channel": format!("邮件 Authorization: Bearer {token}"), "externalReference": format!("https://ops:{secret}@ticket.example.test/1")})).unwrap();
    // 同一载荷重放仍按原记录返回（脱敏不破坏幂等）。
    let replay = act(&connection, pid, &json!({"operationId": "op-rem-secret-02", "action": "record_action", "incidentId": incident, "expectedRevision": result["previousRevision"], "actor": "张三", "reason": format!("通知时对方回了 password={secret}，请轮换"), "handling": "notification_recorded", "channel": format!("邮件 Authorization: Bearer {token}"), "externalReference": format!("https://ops:{secret}@ticket.example.test/1")})).unwrap();
    assert_eq!(replay["replayed"], true);
    let stored = rem_everything_stored(&connection);
    assert!(!stored.contains(secret), "口令原文落库了");
    assert!(!stored.contains(token), "token 原文落库了");
    assert!(stored.contains("Zq7***"), "脱敏后的形态仍可对账");
    let _ = std::fs::remove_dir_all(root);
}

fn rem_everything_stored(connection: &Connection) -> String {
    let tables: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='table'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .flatten()
        .collect();
    let mut all = String::new();
    for table in tables {
        let mut statement = connection.prepare(&format!("SELECT * FROM \"{table}\"")).unwrap();
        let columns = statement.column_count();
        let mut rows = statement.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            for index in 0..columns {
                if let Ok(rusqlite::types::ValueRef::Text(text)) = row.get_ref(index) {
                    all.push_str(&String::from_utf8_lossy(text));
                    all.push('\n');
                }
            }
        }
    }
    all
}

/// P1-6：同一来源里有一部分工作项未配置、或工作项“完成”但来源结果只是部分/限流时，格子不能变绿。
#[test]
fn rem_p1_6_partly_unconfigured_or_partial_source_results_never_turn_green() {
    let root = sandbox("rem-green");
    let (connection, _db) = database(&root);
    let pid = 474;
    project(&connection, pid);
    let mut value = ExposureSettings::for_project(pid);
    value.sources = vec!["passive_web".into(), "certificate_transparency".into(), "github_repositories".into(), "web_archive".into()];
    save(&connection, &value, 0).unwrap();
    let r = run(&connection, pid, "completed");
    // 证书：一个域名查完了，另两个因为缺配置被跳过。
    work_item(&connection, r, pid, "certificate_transparency", "c1", "completed");
    work_item(&connection, r, pid, "certificate_transparency", "c2", "skipped_unconfigured");
    work_item(&connection, r, pid, "certificate_transparency", "c3", "skipped_unconfigured");
    // 代码平台：工作项完成，但来源结果是 partial（incomplete_results / 超出窗口）。
    work_item(&connection, r, pid, "github_repositories", "g1", "completed");
    connection
        .execute("INSERT INTO exposure_source_results(run_id,source_key,status,item_count,error) VALUES(?1,'github_repositories','partial',0,'GitHub 标记 incomplete_results')", [r])
        .unwrap();
    // 归档：工作项完成，来源被限流。
    work_item(&connection, r, pid, "web_archive", "w1", "completed");
    connection
        .execute("INSERT INTO exposure_source_results(run_id,source_key,status,item_count,error) VALUES(?1,'web_archive','rate_limited',0,'429')", [r])
        .unwrap();
    // 对照：真正全部完成且来源 completed 的仍是绿色。
    work_item(&connection, r, pid, "passive_web", "p1", "completed");
    connection
        .execute("INSERT INTO exposure_source_results(run_id,source_key,status,item_count,error) VALUES(?1,'passive_web','completed',1,'')", [r])
        .unwrap();
    let matrix = super::coverage::coverage_matrix(&connection, pid, None, 0).unwrap();
    let row = |capability: &str| matrix["rows"].as_array().unwrap().iter().find(|row| row["capability"] == capability).unwrap().clone();
    for capability in ["certificate_index", "repository_metadata", "archive_index"] {
        let current = row(capability);
        assert_eq!(current["state"], "partial", "{capability}: {}", current["reason"]);
        assert!(current["cells"].as_array().unwrap().iter().all(|cell| cell["tone"] != "green"), "{capability} 有缺口却出现绿格");
    }
    assert_eq!(row("certificate_index")["denominator"]["skippedUnconfigured"], 2);
    let web = row("first_party_web");
    assert_eq!(web["state"], "executed");
    assert!(web["cells"].as_array().unwrap().iter().any(|cell| cell["tone"] == "green"), "真正完成的来源仍可为绿");
    let _ = std::fs::remove_dir_all(root);
}

/// P1 UI 闭环：覆盖矩阵钻取——时间窗口、单位维度（来自归属模块逐资产投影）、缺口筛选、键集分页。
#[test]
fn rem_ui_coverage_drilldown_has_window_unit_and_paged_items() {
    use super::coverage::coverage_drilldown;
    let root = sandbox("rem-drill");
    let (connection, _db) = database(&root);
    let pid = 476;
    directory(&connection);
    for origin in ["https://a.example.test", "https://b.example.test", "https://c.example.test"] {
        seed_scope(&connection, pid, origin, true);
    }
    connection
        .execute("INSERT INTO project_org_bindings(project_id,project_kind,group_entity_id,binding_version) VALUES(?1,'group_ledger',1,3)", [pid])
        .unwrap();
    let asset = |key: &str| -> i64 { connection.query_row("SELECT id FROM assets WHERE asset_key=?1", [key], |r| r.get(0)).unwrap() };
    let (a, b, c) = (asset("https://a.example.test"), asset("https://b.example.test"), asset("https://c.example.test"));
    ownership_group_decision(&connection, pid, a, 5, "op-rem-drill-a1", "consistent");
    ownership_group_decision(&connection, pid, b, 4, "op-rem-drill-b1", "consistent");
    let item = |run_id: i64, key: &str, asset_id: i64, status: &str| {
        connection
            .execute(
                "INSERT INTO exposure_work_items(run_id,project_id,work_key,kind,source_key,asset_id,status,reason) VALUES(?1,?2,?3,'passive_web','passive_web',?4,?5,?6)",
                params![run_id, pid, key, asset_id, status, format!("{status} 的原因")],
            )
            .unwrap();
    };
    let old = run(&connection, pid, "completed");
    connection.execute("UPDATE exposure_runs SET created_at=datetime('now','localtime','-40 days') WHERE id=?1", [old]).unwrap();
    item(old, "o-a", a, "failed");
    let latest = run(&connection, pid, "partial");
    item(latest, "l-a", a, "completed");
    for index in 0..3 {
        item(latest, &format!("l-b-{index}"), b, if index == 0 { "completed" } else { "blocked_quota" });
    }
    item(latest, "l-c", c, "failed");
    // 最近一轮：5 个工作项，按单位分摊：杭州 1/1、咪咕杭州 1/3、未归属 0/1。
    let view = coverage_drilldown(&connection, pid, "passive_web", 0, None, false, 2, "").unwrap();
    assert_eq!(view["window"]["runIds"], json!([latest]));
    assert_eq!((view["total"].as_i64(), view["hasMore"].as_bool(), view["items"].as_array().unwrap().len()), (Some(5), Some(true), 2));
    let units: Vec<(i64, i64, i64)> = view["unitAxis"]["rows"].as_array().unwrap().iter().map(|row| (row["entityId"].as_i64().unwrap(), row["total"].as_i64().unwrap(), row["gaps"].as_i64().unwrap())).collect();
    assert_eq!(units, vec![(0, 1, 1), (4, 3, 2), (5, 1, 0)], "{}", view["unitAxis"]);
    // 翻页读完，不重复不遗漏。
    let mut seen: Vec<i64> = view["items"].as_array().unwrap().iter().map(|row| row["id"].as_i64().unwrap()).collect();
    let mut cursor = view["nextCursor"].as_str().unwrap().to_string();
    while !cursor.is_empty() {
        let page = coverage_drilldown(&connection, pid, "passive_web", 0, None, false, 2, &cursor).unwrap();
        seen.extend(page["items"].as_array().unwrap().iter().map(|row| row["id"].as_i64().unwrap()));
        cursor = page["nextCursor"].as_str().unwrap().to_string();
    }
    assert_eq!(seen.len(), 5);
    // 游标不能换筛选条件复用。
    let mismatch = coverage_drilldown(&connection, pid, "passive_web", 0, None, true, 2, view["nextCursor"].as_str().unwrap()).unwrap_err();
    assert!(mismatch.contains("exposure_cursor_mismatch"), "{mismatch}");
    // 单位筛选 + 只看缺口：咪咕杭州分公司的 2 个配额阻塞。
    let migu = coverage_drilldown(&connection, pid, "passive_web", 0, Some(4), true, 50, "").unwrap();
    assert_eq!(migu["items"].as_array().unwrap().iter().map(|row| row["status"].as_str().unwrap()).collect::<Vec<_>>(), vec!["blocked_quota", "blocked_quota"]);
    let unassigned = coverage_drilldown(&connection, pid, "passive_web", 0, Some(0), false, 50, "").unwrap();
    assert_eq!(unassigned["items"][0]["assetId"], c);
    // 时间窗口：最近 30 天只有最新一轮；最近 60 天包含 40 天前那一轮，各轮单列。
    let month = coverage_drilldown(&connection, pid, "passive_web", 30, None, false, 50, "").unwrap();
    assert_eq!(month["window"]["runIds"], json!([latest]));
    let two_months = coverage_drilldown(&connection, pid, "passive_web", 60, None, false, 50, "").unwrap();
    assert_eq!(two_months["window"]["runIds"], json!([latest, old]));
    assert_eq!(two_months["total"], 6);
    assert_eq!(two_months["rounds"][1]["itemCounts"]["failed"], 1);
    // 外部来源不按资产计划：单位维度如实说明，按单位筛选被拒绝。
    work_item(&connection, latest, pid, "certificate_transparency", "ct-1", "completed");
    let ct = coverage_drilldown(&connection, pid, "certificate_transparency", 0, None, false, 50, "").unwrap();
    assert_eq!(ct["unitAxis"]["state"], "not_asset_scoped");
    assert!(coverage_drilldown(&connection, pid, "certificate_transparency", 0, Some(4), false, 50, "").unwrap_err().contains("exposure_coverage_unit_unsupported"));
    assert!(coverage_drilldown(&connection, pid, "nope", 0, None, false, 50, "").unwrap_err().contains("exposure_coverage_source_invalid"));
    assert!(coverage_drilldown(&connection, pid, "passive_web", 400, None, false, 50, "").unwrap_err().contains("exposure_coverage_window_invalid"));
    let _ = std::fs::remove_dir_all(root);
}

/// 规模修复（P1-REM）：整项目投影一次读出归属事件，必须与逐资产查询逐条一致——包括组决定之后
/// 改过归属、决定事件缺失、`LIKE` 的大小写规则和决定之前的旧变更。
#[test]
fn rem_scale_full_projection_matches_the_per_asset_edges_exactly() {
    let root = sandbox("rem-proj-eq");
    let (connection, _db) = database(&root);
    let pid = 478;
    directory(&connection);
    let origins = ["https://a.example.test", "https://b.example.test", "https://c.example.test", "https://d.example.test", "https://e.example.test", "https://f.example.test"];
    for origin in origins {
        seed_scope(&connection, pid, origin, true);
    }
    connection
        .execute("INSERT INTO project_org_bindings(project_id,project_kind,group_entity_id,binding_version) VALUES(?1,'group_ledger',1,3)", [pid])
        .unwrap();
    let asset = |key: &str| -> i64 { connection.query_row("SELECT id FROM assets WHERE asset_key=?1", [key], |r| r.get(0)).unwrap() };
    let ids: Vec<i64> = origins.iter().map(|origin| asset(origin)).collect();
    let event = |asset_id: i64, summary: &str| {
        connection
            .execute("INSERT INTO asset_events(project_id,asset_id,event_type,summary) VALUES(?1,?2,'ownership',?3)", params![pid, asset_id, summary])
            .unwrap();
    };
    // d：决定之前的旧变更不影响。
    event(ids[3], "attributed: 自动判断");
    for (index, asset_id) in ids.iter().enumerate() {
        ownership_group_decision(&connection, pid, *asset_id, if index % 2 == 0 { 5 } else { 4 }, &format!("op-rem-eq-{index}"), "consistent");
    }
    // b：组决定之后人工改过（即使之后又单独确认）。
    event(ids[1], "revoked: 人工撤销");
    event(ids[1], "confirmed: 单独确认");
    // c：前缀大小写不同，SQLite LIKE 视为相同。
    connection.execute("UPDATE asset_events SET summary='CONFIRMED:'||substr(summary,11) WHERE asset_id=?1 AND summary LIKE 'confirmed:%'", [ids[2]]).unwrap();
    // e：决定事件缺失。
    connection.execute("DELETE FROM asset_events WHERE asset_id=?1", [ids[4]]).unwrap();
    // f：大写的非确认事件（如 'Revoked:'）也算变更；以 'Confirmed' 开头但没有冒号的不算变更、也不算决定。
    event(ids[5], "Confirmedish note");
    let full = crate::commands::ownership_asset_unit_projection(&connection, pid).unwrap();
    let mut single_edges = Vec::new();
    for asset_id in &ids {
        if let Some(edge) = crate::commands::ownership_asset_unit_edge(&connection, pid, *asset_id).unwrap() {
            single_edges.push((edge.asset_id, edge.owner_entity_id, edge.version.clone()));
        }
    }
    let full_edges: Vec<(i64, i64, String)> = full.edges.iter().map(|edge| (edge.asset_id, edge.owner_entity_id, edge.version.clone())).collect();
    assert_eq!(full_edges, single_edges, "整项目投影与逐资产查询逐条一致");
    let with_edge: Vec<i64> = full_edges.iter().map(|row| row.0).collect();
    assert_eq!(with_edge, vec![ids[0], ids[2], ids[3], ids[5]], "{:?}", full.unassigned_reasons);
    assert_eq!(full.unassigned_reasons.get("changed_after_group_decision"), Some(&1));
    assert_eq!(full.unassigned_reasons.get("decision_event_missing"), Some(&1));
    let _ = std::fs::remove_dir_all(root);
}

// ───────────────────────────── FIX3（2026-10-09 17:29） ─────────────────────────────

/// FIX3-R5：复测已 404，但关闭前又出现了新位置。旧复测不能关闭（零写入），重新复测覆盖新位置后才行。
#[test]
fn fix3_r5_a_new_location_after_the_retest_blocks_closing_until_a_new_retest() {
    let root = sandbox("fix3-r5");
    let (connection, _db) = database(&root);
    let pid = 481;
    let url = "https://p481.example.test/a";
    let fresh = "https://p481.example.test/b";
    let r1 = run(&connection, pid, "completed");
    let (finding, _) = add(&connection, pid, r1, &lead(url, "疑似密钥").body("LEAK481"));
    propose(&connection, pid, Some(r1)).unwrap();
    let incident = incident_of(&connection, finding).unwrap();
    act(&connection, pid, &review(incident, revision(&connection, incident), "verified", "op-f3r5-01")).unwrap();
    let gone = run(&connection, pid, "completed");
    let mut removed = lead(url, "疑似密钥").body("NOT-FOUND");
    removed.status_code = 404;
    add(&connection, pid, gone, &removed);
    act(&connection, pid, &rem_link_retest(incident, revision(&connection, incident), gone, "op-f3r5-02")).unwrap();
    // 复测之后、关闭之前：同一份内容出现在新位置。
    let later = run(&connection, pid, "completed");
    let (other, _) = add(&connection, pid, later, &lead(fresh, "疑似密钥").body("LEAK481"));
    propose(&connection, pid, Some(later)).unwrap();
    assert_eq!(incident_of(&connection, other), Some(incident), "新位置并入同一事件");
    let before = (count(&connection, "exposure_incident_events"), revision(&connection, incident));
    let refused = act(&connection, pid, &rem_close(incident, revision(&connection, incident), "op-f3r5-03")).unwrap_err();
    assert!(refused.contains("exposure_incident_retest_stale") && refused.contains(fresh), "{refused}");
    assert_eq!((count(&connection, "exposure_incident_events"), revision(&connection, incident)), before, "拒绝关闭零写入");
    // 新复测覆盖两个位置且都 404：可以关闭。
    let again = run(&connection, pid, "completed");
    for target in [url, fresh] {
        let mut gone_again = lead(target, "疑似密钥").body("NOT-FOUND");
        gone_again.status_code = 404;
        add(&connection, pid, again, &gone_again);
    }
    act(&connection, pid, &rem_link_retest(incident, revision(&connection, incident), again, "op-f3r5-04")).unwrap();
    let closed = act(&connection, pid, &rem_close(incident, revision(&connection, incident), "op-f3r5-05")).unwrap();
    assert_eq!(closed["closureBasis"], "manual_with_retest");
    let _ = std::fs::remove_dir_all(root);
}

/// FIX3-R6：两段不同的秘密脱敏后相同（同前缀/后缀/长度）。同一 operationId 换一个秘密不能被当作重放，
/// 返回旧结果；必须冲突且零写入。真正的重放仍然返回首个结果，且首次/重放响应都不含原值。
#[test]
fn fix3_r6_a_redaction_collision_is_a_different_payload_not_a_replay() {
    let first_secret = "SENTINELa1b2c3d4ZZ";
    let second_secret = "SENTINELx9y8w7v6ZZ";
    assert_eq!(
        super::redaction::redact_text(&format!("password={first_secret}")),
        super::redaction::redact_text(&format!("password={second_secret}")),
        "前提：两段秘密脱敏后相同"
    );
    let root = sandbox("fix3-r6");
    let (connection, _db) = database(&root);
    let pid = 482;
    let r1 = run(&connection, pid, "completed");
    let (finding, _) = add(&connection, pid, r1, &lead("https://p482.example.test/a", "疑似密钥").body("LEAK482"));
    propose(&connection, pid, Some(r1)).unwrap();
    let incident = incident_of(&connection, finding).unwrap();
    let expected = revision(&connection, incident);
    let with = |secret: &str| {
        let mut body = review(incident, expected, "verified", "op-f3r6-001");
        body["reason"] = json!(format!("核实 password={secret} 已在页面出现"));
        body
    };
    let first = act(&connection, pid, &with(first_secret)).unwrap();
    let after = (count(&connection, "exposure_incident_events"), revision(&connection, incident));
    let replay = act(&connection, pid, &with(first_secret)).unwrap();
    assert_eq!(replay["replayed"], true);
    let collided = act(&connection, pid, &with(second_secret)).unwrap_err();
    assert!(collided.contains("exposure_incident_operation_conflict"), "脱敏碰撞不能被当作重放：{collided}");
    assert_eq!((count(&connection, "exposure_incident_events"), revision(&connection, incident)), after, "冲突零写入");
    for (label, value) in [("首次", first.to_string()), ("重放", replay.to_string()), ("冲突", collided.clone())] {
        for secret in [first_secret, second_secret] {
            assert!(!value.contains(secret), "{label}响应含原值：{value}");
        }
    }
    let _ = std::fs::remove_dir_all(root);
}

// ───────────────────────────── FIX4（2026-10-09 原六项） ─────────────────────────────

/// FIX4 P1／R3：只完成 1 项、计划外 900 项时覆盖不能变绿；待分类的凭据线索不能显示为“已执行无命中”。
#[test]
fn fix4_r3_outside_plan_gaps_and_classification_pending_leads_never_turn_green() {
    let root = sandbox("fix4-r3");
    let (connection, _db) = database(&root);
    let pid = 597;
    project(&connection, pid);
    let mut value = ExposureSettings::for_project(pid);
    value.sources = vec!["passive_web".into()];
    save(&connection, &value, 0).unwrap();
    let r = run(&connection, pid, "completed");
    work_item(&connection, r, pid, "passive_web", "p1", "completed");
    let web = |matrix: &JsonValue| matrix["rows"].as_array().unwrap().iter().find(|row| row["capability"] == "first_party_web").unwrap().clone();
    let matrix = super::coverage::coverage_matrix(&connection, pid, None, 900).unwrap();
    let row = web(&matrix);
    let green: Vec<String> = row["cells"].as_array().unwrap().iter().filter(|cell| cell["tone"] == "green").map(|cell| cell["infoCategory"].as_str().unwrap().to_string()).collect();
    assert!(green.is_empty(), "完成 1 项、计划外 900 项仍有 {} 个绿格子：{green:?}（行状态 {}）", green.len(), row["state"]);
    assert_ne!(row["state"], "executed");
    assert!(row["reason"].as_str().unwrap().contains("900"), "{}", row["reason"]);
    // 计划外为 0 时，同一轮仍可以是“已执行”（对照）。
    assert_eq!(web(&super::coverage::coverage_matrix(&connection, pid, None, 0).unwrap())["state"], "executed");
    // 一条待分类的凭据线索。
    add(&connection, pid, r, &lead("https://a.example.test/.env", "疑似密钥").body("K"));
    connection
        .execute("UPDATE exposure_finding_dispositions SET disposition='classification_pending' WHERE project_id=?1", [pid])
        .unwrap();
    let pending: i64 = connection
        .query_row("SELECT COUNT(*) FROM exposure_finding_dispositions WHERE project_id=?1 AND disposition='classification_pending' AND info_category='credential'", [pid], |row| row.get(0))
        .unwrap();
    assert_eq!(pending, 1);
    let row = web(&super::coverage::coverage_matrix(&connection, pid, None, 0).unwrap());
    let credential = row["cells"].as_array().unwrap().iter().find(|cell| cell["infoCategory"] == "credential").unwrap().clone();
    assert_ne!(credential["state"], "executed_no_hit", "待分类的凭据线索显示成了“{}”", credential["label"]);
    assert_ne!(credential["tone"], "green");
    let _ = std::fs::remove_dir_all(root);
}
