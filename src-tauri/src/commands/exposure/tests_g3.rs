// G3 — documents and intel. File import (G3-0), explicit public document URLs through the
// production worker (G3-1), user-supplied intel reports with grades and the manual grade
// change (G3-2), classification reuse and same_document candidates (G3-3), and the closed
// loop (G3-4). Network tests talk to localhost fixtures only; the single live read of a public
// document is `#[ignore]` and runs only when asked for explicitly.

use super::adapters::documents::{self, execute_link, LinkContext, KIND_DOCUMENT_LINK, UNCONFIGURED_ENTITY};
use super::adapters::github::{CredentialLookup, GithubEndpoint};
use super::adapters::http::{PassiveClient, PassiveHttpLimits, RequestMeter};
use super::fixture::{response, routes, seed_scope, Fixture};
use super::imports::{grade_intel_finding, import_batch};
use super::orchestrator::{start_or_reuse, ItemOutcome};
use super::settings::{save, DocumentUrl, ExposureSettings};
use super::state::{ensure_exposure_schema, SourceStatus, WorkItemStatus};
use super::work_items::WorkItem;
use base64::Engine;
use rusqlite::{params, Connection};
use serde_json::{json, Value as JsonValue};
use std::path::{Path, PathBuf};

fn sandbox(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("oviraptor-exposure-g3-{tag}-{}", uuid::Uuid::new_v4()))
}

fn database(root: &Path) -> (Connection, PathBuf) {
    let db_path = crate::db::initialize(&root.join("app")).expect("临时库");
    let connection = crate::db::open(&db_path).expect("连接");
    ensure_exposure_schema(&connection).expect("附加结构必须可用");
    (connection, db_path)
}

fn client(max_response_bytes: usize) -> PassiveClient {
    PassiveClient::new(PassiveHttpLimits {
        timeout: std::time::Duration::from_secs(5),
        max_response_bytes,
        ..PassiveHttpLimits::default()
    })
    .expect("客户端必须可用")
}

fn meter(max_requests: i64) -> RequestMeter {
    RequestMeter { requests_used: 0, bytes_used: 0, max_requests, max_bytes: 50_000_000 }
}

fn never() -> bool {
    false
}

fn always() -> bool {
    true
}

fn link_item(entity: &str) -> WorkItem {
    WorkItem {
        id: 1,
        run_id: 1,
        project_id: 301,
        work_key: format!("{KIND_DOCUMENT_LINK}|{entity}"),
        kind: KIND_DOCUMENT_LINK.to_string(),
        source_key: KIND_DOCUMENT_LINK.to_string(),
        asset_id: None,
        origin: String::new(),
        entity: entity.to_string(),
        input_hash: String::new(),
        status: WorkItemStatus::Pending,
        owner: String::new(),
        claim_version: 0,
        attempts: 0,
        observations: 0,
        requests: 0,
        bytes: 0,
    }
}

fn entry(url: &str) -> DocumentUrl {
    DocumentUrl { url: documents::canonical_document_url(url).unwrap(), public_material: false, note: String::new() }
}

fn terms() -> Vec<String> {
    vec!["example.test".to_string(), "样本公司".to_string()]
}

fn read_link(fixture_client: &PassiveClient, url: &str, allow_private: bool, any_port: bool, meter: &mut RequestMeter) -> ItemOutcome {
    let terms = terms();
    let context = LinkContext { client: fixture_client, terms: &terms, allow_private, any_port, cancelled: &never };
    let target = entry(url);
    execute_link(&context, &link_item(&target.url), meter, Some(&target))
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// A project with a confirmed profile: approved domain `example.test`, legal name 样本公司.
fn project(connection: &Connection, project_id: i64, site_origin: &str) {
    seed_scope(connection, project_id, site_origin, true);
    connection
        .execute(
            "UPDATE asset_ownership_profiles SET approved_domains_json=?2 WHERE project_id=?1",
            params![project_id, r#"["example.test"]"#],
        )
        .unwrap();
}

fn import_request(operation_id: &str, channel: &str, files: &[(&str, &str, &[u8])]) -> JsonValue {
    json!({
        "operationId": operation_id,
        "channel": channel,
        "operator": "王五",
        "sourceDeclaration": "用户从内部共享盘导出后提供",
        "files": files.iter().map(|(name, format, bytes)| json!({"name": name, "format": format, "contentBase64": b64(bytes)})).collect::<Vec<_>>(),
    })
}

fn count(connection: &Connection, table: &str) -> i64 {
    connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0)).unwrap()
}

/// Rows a refusal must leave untouched, plus the shared tables an import must never write.
fn footprint(connection: &Connection) -> Vec<(String, i64)> {
    [
        "exposure_runs",
        "exposure_import_batches",
        "exposure_observations",
        "exposure_findings",
        "exposure_work_items",
        "exposure_intel_grades",
        "assets",
        "org_entities",
        "asset_ownership_decisions",
    ]
    .iter()
    .map(|table| (table.to_string(), count(connection, table)))
    .collect()
}

fn observation(connection: &Connection, run_id: i64, url: &str) -> (String, String, i64, i64, JsonValue) {
    let (method, kind, status, truncated, evidence): (String, String, i64, i64, String) = connection
        .query_row(
            "SELECT method,kind,status_code,truncated,evidence_json FROM exposure_observations WHERE run_id=?1 AND url=?2 ORDER BY id LIMIT 1",
            params![run_id, url],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .unwrap_or_else(|error| panic!("第 {run_id} 轮没有 {url} 的观察：{error}"));
    let parsed: JsonValue = serde_json::from_str(&evidence).unwrap();
    (method, kind, status, truncated, parsed["detail"].clone())
}

fn finding_id(connection: &Connection, project_id: i64, url: &str, title: &str) -> i64 {
    connection
        .query_row(
            "SELECT id FROM exposure_findings WHERE project_id=?1 AND source_url=?2 AND title=?3",
            params![project_id, url, title],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| panic!("没有 {url} / {title} 的线索"))
}

fn disposition(connection: &Connection, project_id: i64, url: &str, title: &str) -> (String, String) {
    let id = finding_id(connection, project_id, url, title);
    let view = super::classify::disposition_of(connection, project_id, id).unwrap().expect("必须有处置");
    (view.disposition.to_string(), view.source_channel.to_string())
}

fn findings_at(connection: &Connection, project_id: i64, url: &str) -> i64 {
    connection
        .query_row("SELECT COUNT(*) FROM exposure_findings WHERE project_id=?1 AND source_url=?2", params![project_id, url], |row| row.get(0))
        .unwrap()
}

fn source_result(connection: &Connection, run_id: i64, source: &str) -> (String, String) {
    connection
        .query_row(
            "SELECT status,error FROM exposure_source_results WHERE run_id=?1 AND source_key=?2",
            params![run_id, source],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap_or_else(|_| ("<missing>".into(), String::new()))
}

fn requests_used(connection: &Connection, run_id: i64) -> i64 {
    connection
        .query_row("SELECT requests_used FROM exposure_run_plans WHERE run_id=?1", [run_id], |row| row.get(0))
        .unwrap()
}

/// Every text cell of every table, for "this text was never stored".
fn everything_stored(connection: &Connection) -> String {
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

fn site() -> Fixture {
    Fixture::start(routes(vec![("/", vec![response(200, &[("Content-Type", "text/html")], "<html><title>首页</title></html>")])]))
}

fn html(body: &str) -> String {
    response(200, &[("Content-Type", "text/html; charset=utf-8")], body)
}

fn document_project(connection: &Connection, project_id: i64, site: &Fixture, sources: &[&str], urls: Vec<DocumentUrl>) -> JsonValue {
    project(connection, project_id, &site.origin);
    let mut value = ExposureSettings::for_project(project_id);
    value.sources = sources.iter().map(|row| row.to_string()).collect();
    value.allow_private_destinations = true;
    value.timeout_seconds = 5;
    value.document_urls = urls;
    let current: i64 = connection
        .query_row("SELECT COALESCE(MAX(version),0) FROM exposure_settings WHERE project_id=?1", [project_id], |row| row.get(0))
        .unwrap();
    save(connection, &value, current).expect("配置必须能保存");
    super::exposure_scan_settings(connection, project_id).expect("配置必须可读")
}

fn fixture_endpoints() -> super::SourceEndpoints {
    super::SourceEndpoints {
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
    super::use_test_endpoints_once(fixture_endpoints());
    super::execute_exposure_run(db_path.to_path_buf(), started.run_id, project_id, "g3-worker".into(), settings.clone());
    started.run_id
}

fn file_url(report: &JsonValue, index: usize) -> String {
    report["files"][index]["url"].as_str().unwrap().to_string()
}

// ───────────────────────────── G3-0 file import ─────────────────────────────

#[test]
fn g3_0_text_json_csv_import_runs_the_production_path_as_leads_not_assets() {
    let root = sandbox("import");
    let (connection, _db) = database(&root);
    let pid = 301;
    project(&connection, pid, "http://127.0.0.1:9");
    let before = footprint(&connection);
    // MIDMARKER sits past the 320-character excerpt window but inside the 400-character cap, so
    // either bound failing alone is visible; TAILMARKER is far beyond both.
    let text = format!("样本公司 内部周报：系统迁移安排。{}MIDMARKER{}TAILMARKER-远端正文不入库", "甲".repeat(330), "填充。".repeat(200));
    let json_doc = json!({"title": "接口清单", "items": [{"host": "api.example.test", "owner": "运维"}]}).to_string();
    let csv_doc = "系统,负责人\n\"门户, 外网\",样本公司 信息部\n";
    let request = import_request(
        "op-import-0001",
        "document_import",
        &[("周报.txt", "text", text.as_bytes()), ("接口.json", "json", json_doc.as_bytes()), ("台账.csv", "csv", csv_doc.as_bytes())],
    );
    let report = import_batch(&connection, pid, &request).expect("导入成功");
    assert_eq!(report["runStatus"], "completed", "{report}");
    assert_eq!(report["replayed"], false);
    assert_eq!(report["declaredUrlVerified"], false);
    let run_id = report["runId"].as_i64().unwrap();
    let batch_id = report["batchId"].as_i64().unwrap();
    for index in 0..3 {
        let row = &report["files"][index];
        assert_eq!(row["result"]["findings"], 1, "{row}");
        assert_eq!(row["result"]["sourceStatus"], "completed", "{row}");
        assert_eq!(row["sha256"].as_str().unwrap().len(), 64);
        let url = file_url(&report, index);
        assert!(url.starts_with("import://op-import-0001/"), "{url}");
        let (method, kind, status, _, detail) = observation(&connection, run_id, &url);
        assert_eq!((method.as_str(), kind.as_str(), status), ("IMPORT", "document_import", 0));
        assert_eq!(detail["collectionMode"], "import");
        assert_eq!(detail["networkObserved"], false);
        assert_eq!(detail["declaredUrlVerified"], false);
        assert_eq!(detail["batchId"], batch_id);
        assert_eq!(detail["operator"], "王五");
        assert_eq!(detail["sourceDeclaration"], "用户从内部共享盘导出后提供");
        assert_eq!(detail["fileSha256"], row["sha256"]);
        assert_eq!(disposition(&connection, pid, &url, documents::TITLE_DOCUMENT_MENTION), ("classification_pending".to_string(), "document_import".to_string()));
    }
    let (_, _, _, _, json_detail) = observation(&connection, run_id, &file_url(&report, 1));
    assert_eq!(json_detail["mentions"], json!(["example.test"]));
    let (_, _, _, _, csv_detail) = observation(&connection, run_id, &file_url(&report, 2));
    assert_eq!(csv_detail["mentions"], json!(["样本公司"]));
    assert_eq!(source_result(&connection, run_id, "document_import").0, "completed");
    // Only exposure tables moved: no asset, entity or ownership decision was created.
    let after = footprint(&connection);
    for (table, rows) in &before {
        if ["assets", "org_entities", "asset_ownership_decisions"].contains(&table.as_str()) {
            assert_eq!(after.iter().find(|(name, _)| name == table).unwrap().1, *rows, "{table} 不能被导入改写");
        }
    }
    // The body is never stored: text far from the mention is not in any table.
    let stored = everything_stored(&connection);
    assert!(!stored.contains("TAILMARKER"), "正文不入库");
    assert!(!stored.contains("MIDMARKER"), "摘录只取命中附近的窗口");
    let (operator, declaration, status, version): (String, String, String, String) = connection
        .query_row(
            "SELECT operator,source_declaration,status,import_version FROM exposure_import_batches WHERE id=?1",
            [batch_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!((operator.as_str(), status.as_str()), ("王五", "completed"));
    assert_eq!(declaration, "用户从内部共享盘导出后提供");
    assert_eq!(version, super::imports::IMPORT_VERSION);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g3_0_pdf_office_images_bad_text_are_named_gaps_never_parsed() {
    let root = sandbox("unsupported");
    let (connection, _db) = database(&root);
    let pid = 302;
    project(&connection, pid, "http://127.0.0.1:9");
    let files: Vec<(&str, &str, &[u8])> = vec![
        ("报告.pdf", "text", b"%PDF-1.7\n\xE6\xA0\xB7\xE6\x9C\xAC\xE5\x85\xAC\xE5\x8F\xB8"),
        ("表.docx", "text", b"PK\x03\x04 \xE6\xA0\xB7\xE6\x9C\xAC\xE5\x85\xAC\xE5\x8F\xB8"),
        ("图.png", "text", b"\x89PNG\r\n\x1a\n"),
        ("乱码.txt", "text", &[0x66, 0xFF, 0xFE, 0x41]),
        ("坏.json", "json", b"{\"a\":"),
        ("坏.csv", "csv", b"a,b\n\"x,1\n"),
        ("无关.txt", "text", "今天天气不错".as_bytes()),
    ];
    let report = import_batch(&connection, pid, &import_request("op-unsupported-1", "document_import", &files)).expect("批次可写");
    let run_id = report["runId"].as_i64().unwrap();
    let expect = [
        ("unsupported_format", "unsupported"),
        ("unsupported_format", "unsupported"),
        ("unsupported_format", "unsupported"),
        ("invalid_utf8", "parse_error"),
        ("parse_error", "parse_error"),
        ("parse_error", "parse_error"),
        ("", "completed_no_matches"),
    ];
    for (index, (code, source)) in expect.iter().enumerate() {
        let result = &report["files"][index]["result"];
        assert_eq!(result["sourceStatus"], *source, "{index}: {result}");
        assert!(result["reason"].as_str().unwrap().starts_with(code), "{index}: {result}");
        assert_eq!(result["findings"], 0, "{index}");
        let (_, _, _, _, detail) = observation(&connection, run_id, &file_url(&report, index));
        assert_eq!(detail["contentAnalyzed"], *source == "completed_no_matches", "{index}");
    }
    assert!(report["files"][0]["result"]["reason"].as_str().unwrap().contains("pdf"));
    assert!(report["files"][1]["result"]["reason"].as_str().unwrap().contains("zip_or_office"));
    assert_eq!(findings_at(&connection, pid, &file_url(&report, 0)), 0, "PDF 里的集团名称不能被“读出”");
    // The worst item decides the source: a parse error is a gap, never "completed".
    assert_eq!(source_result(&connection, run_id, "document_import").0, "parse_error");
    assert_ne!(report["runStatus"], "completed");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g3_0_invalid_batches_are_refused_before_any_write() {
    let root = sandbox("refuse");
    let (connection, _db) = database(&root);
    let pid = 303;
    project(&connection, pid, "http://127.0.0.1:9");
    connection.execute("INSERT INTO projects(id,name,status) VALUES(304,'archived','archived')", []).unwrap();
    let ok_file: &[(&str, &str, &[u8])] = &[("a.txt", "text", "样本公司".as_bytes())];
    let good = import_request("op-refuse-0001", "document_import", ok_file);
    let with = |key: &str, value: JsonValue| {
        let mut request = good.clone();
        request[key] = value;
        request
    };
    let big = vec![b'a'; super::imports::IMPORT_FILE_MAX_BYTES + 1];
    let many: Vec<(&str, &str, &[u8])> = (0..21).map(|_| ("a.txt", "text", "x".as_bytes())).collect();
    let cases: Vec<(&str, i64, JsonValue)> = vec![
        ("operationId", pid, with("operationId", json!("short"))),
        ("channel", pid, with("channel", json!("netdisk_search"))),
        ("operator", pid, with("operator", json!("  "))),
        ("declaration", pid, with("sourceDeclaration", json!(""))),
        ("unknown field", pid, with("autoConfirm", json!(true))),
        ("21 files", pid, import_request("op-refuse-0001", "document_import", &many)),
        ("big file", pid, import_request("op-refuse-0001", "document_import", &[("big.txt", "text", &big)])),
        ("path name", pid, import_request("op-refuse-0001", "document_import", &[("../etc/passwd", "text", b"x")])),
        ("bad base64", pid, json!({"operationId": "op-refuse-0001", "channel": "document_import", "operator": "王五", "sourceDeclaration": "x", "files": [{"name": "a.txt", "format": "text", "contentBase64": "@@@"}]})),
        ("bad format", pid, import_request("op-refuse-0001", "document_import", &[("a.pdf", "pdf", b"x")])),
        ("intel text", pid, import_request("op-refuse-0001", "public_intel_import", &[("a.txt", "text", b"x")])),
        ("public no note", pid, with("publicMaterial", json!(true))),
        ("intel public", pid, {
            let mut request = import_request("op-refuse-0001", "public_intel_import", &[("a.json", "json", b"[]")]);
            request["publicMaterial"] = json!(true);
            request["publicMaterialNote"] = json!("公开");
            request
        }),
        ("declared ftp", pid, with("declaredUrl", json!("ftp://files.example.test/a"))),
        ("no project", 999, good.clone()),
        ("archived project", 304, good.clone()),
    ];
    let before = footprint(&connection);
    for (label, project_id, request) in cases {
        let error = import_batch(&connection, project_id, &request).expect_err(label);
        let code = if label.ends_with("project") { "[exposure_import_project]" } else { "[exposure_import_invalid]" };
        assert!(error.starts_with(code), "{label}: {error}");
        assert_eq!(footprint(&connection), before, "{label} 必须零写");
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g3_0_operation_id_replays_one_payload_refuses_another_and_waits_for_a_running_round() {
    let root = sandbox("replay");
    let (connection, _db) = database(&root);
    let pid = 305;
    project(&connection, pid, "http://127.0.0.1:9");
    let request = import_request("op-replay-0001", "document_import", &[("a.txt", "text", "样本公司 资料".as_bytes())]);
    let first = import_batch(&connection, pid, &request).unwrap();
    let after_first = footprint(&connection);
    let again = import_batch(&connection, pid, &request).unwrap();
    assert_eq!(again["replayed"], true);
    assert_eq!(again["runId"], first["runId"]);
    assert_eq!(again["batchId"], first["batchId"]);
    assert_eq!(footprint(&connection), after_first, "重放不能多写");
    let other = import_request("op-replay-0001", "document_import", &[("a.txt", "text", "样本公司 另一份".as_bytes())]);
    let error = import_batch(&connection, pid, &other).unwrap_err();
    assert!(error.starts_with("[exposure_import_operation_conflict]"), "{error}");
    assert_eq!(footprint(&connection), after_first);
    // An active round blocks the import before anything is written.
    connection
        .execute("INSERT INTO exposure_runs(project_id,status,stage,created_at) VALUES(?1,'running','executing','2026-10-09 01:00:00')", [pid])
        .unwrap();
    let busy_before = footprint(&connection);
    let error = import_batch(&connection, pid, &import_request("op-replay-0002", "document_import", &[("b.txt", "text", b"x")])).unwrap_err();
    assert!(error.starts_with("[exposure_import_busy]"), "{error}");
    assert_eq!(footprint(&connection), busy_before);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g3_0_declared_url_is_a_claim_never_fetched_and_extracted_text_is_labelled() {
    let root = sandbox("declared");
    let (connection, _db) = database(&root);
    let pid = 306;
    project(&connection, pid, "http://127.0.0.1:9");
    let origin = Fixture::start(routes(vec![("/source.pdf", vec![response(200, &[], "%PDF-1.7")])]));
    let mut request = import_request("op-declared-01", "document_import", &[("提取.txt", "extracted_text", "样本公司 年度网络规划（节选）".as_bytes())]);
    request["declaredUrl"] = json!(origin.url("/source.pdf#page=2"));
    request["extractedFrom"] = json!("pdf");
    let report = import_batch(&connection, pid, &request).unwrap();
    assert_eq!(origin.request_count(), 0, "声明的来源 URL 不会被访问");
    assert_eq!(report["declaredUrl"], origin.url("/source.pdf"));
    let url = file_url(&report, 0);
    let (_, _, _, _, detail) = observation(&connection, report["runId"].as_i64().unwrap(), &url);
    assert_eq!(detail["collectionMode"], "import_extracted_text");
    assert_eq!(detail["extractedFrom"], "pdf");
    assert_eq!(detail["declaredUrlVerified"], false);
    let id = finding_id(&connection, pid, &url, documents::TITLE_DOCUMENT_MENTION);
    let evidence: String = connection.query_row("SELECT evidence_excerpt FROM exposure_findings WHERE id=?1", [id], |row| row.get(0)).unwrap();
    assert!(evidence.contains("正文由导入方提取") && evidence.contains("不是应用从外网观察到的"), "{evidence}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g3_0_an_interrupted_import_is_not_resumed_as_if_the_text_were_still_there() {
    let root = sandbox("resume");
    let (connection, db_path) = database(&root);
    let pid = 307;
    project(&connection, pid, "http://127.0.0.1:9");
    let report = import_batch(&connection, pid, &import_request("op-resume-0001", "document_import", &[("a.txt", "text", "样本公司".as_bytes())])).unwrap();
    let run_id = report["runId"].as_i64().unwrap();
    // Simulate a crash before the item closed and a later resume by the network worker.
    connection.execute("UPDATE exposure_runs SET status='running' WHERE id=?1", [run_id]).unwrap();
    connection.execute("UPDATE exposure_work_items SET status='pending',owner='',claim_version=claim_version+1 WHERE run_id=?1", [run_id]).unwrap();
    connection.execute("UPDATE exposure_import_batches SET status='running' WHERE run_id=?1", [run_id]).unwrap();
    let settings = super::exposure_scan_settings(&connection, pid).unwrap();
    super::use_test_endpoints_once(fixture_endpoints());
    super::execute_exposure_run(db_path.clone(), run_id, pid, "g3-resume".into(), settings);
    let (status, reason): (String, String) = connection
        .query_row("SELECT status,reason FROM exposure_work_items WHERE run_id=?1", [run_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    assert_eq!(status, "skipped_unconfigured");
    assert!(reason.starts_with("import_not_resumable"), "{reason}");
    let error = import_batch(&connection, pid, &import_request("op-resume-0001", "document_import", &[("a.txt", "text", "样本公司".as_bytes())])).unwrap_err();
    assert!(error.starts_with("[exposure_import_interrupted]"), "{error}");
    let _ = std::fs::remove_dir_all(root);
}

// ───────────────────────────── G3-1 explicit document URL ─────────────────────────────

#[test]
fn g3_1_explicit_documents_are_read_once_through_the_production_worker() {
    let site = site();
    let docs = Fixture::start(routes(vec![
        ("/standard.html", vec![html("<html><head><title>接口规范 v2</title><script>var owner='样本公司';</script><style>.x{}</style></head><body><h1>接口规范</h1><p>适用于 api.example.test 的对接方 &amp; 合作伙伴。</p><!-- 样本公司 内部注释 --></body></html>")]),
        ("/notice.txt", vec![response(200, &[("Content-Type", "text/plain; charset=utf-8")], "样本公司 发布的维护通知")]),
        ("/script-only.html", vec![html("<html><body><script>var x='样本公司';</script><p>无关内容</p></body></html>")]),
    ]));
    let root = sandbox("link");
    let (connection, db_path) = database(&root);
    let pid = 311;
    let urls: Vec<DocumentUrl> = ["/standard.html", "/notice.txt", "/script-only.html"].iter().map(|path| entry(&docs.url(path))).collect();
    let settings = document_project(&connection, pid, &site, &["passive_web", "document_link"], urls);
    let run_id = run_round(&connection, &db_path, pid, &settings);
    let standard = docs.url("/standard.html");
    let (method, kind, status, truncated, detail) = observation(&connection, run_id, &standard);
    assert_eq!((method.as_str(), kind.as_str(), status, truncated), ("GET", "document_read", 200, 0));
    assert_eq!(detail["collectionMode"], "http_read");
    assert_eq!(detail["networkObserved"], true);
    assert_eq!(detail["contentAnalyzed"], true);
    assert_eq!(detail["title"], "接口规范 v2");
    // Scripts, styles and comments are dropped: only the visible text is scanned.
    assert_eq!(detail["mentions"], json!(["example.test"]));
    assert!(detail["normalizedTextSha256"].as_str().is_some_and(|hash| hash.len() == 64));
    let (_, _, _, _, notice) = observation(&connection, run_id, &docs.url("/notice.txt"));
    assert_eq!(notice["mentions"], json!(["样本公司"]));
    let script = docs.url("/script-only.html");
    let (_, _, _, _, script_detail) = observation(&connection, run_id, &script);
    assert_eq!(script_detail["mentions"], json!([]));
    assert_eq!(findings_at(&connection, pid, &script), 0);
    for url in [&standard, &docs.url("/notice.txt")] {
        assert_eq!(disposition(&connection, pid, url, documents::TITLE_DOCUMENT_MENTION), ("classification_pending".to_string(), "document_link".to_string()));
    }
    assert_eq!(source_result(&connection, run_id, "document_link").0, "completed");
    let mut seen = docs.requests();
    seen.sort();
    assert_eq!(seen, vec!["/notice.txt", "/script-only.html", "/standard.html"], "每个地址只 GET 一次");
    assert!(docs.heads().iter().all(|head| head.to_ascii_lowercase().contains("accept: text/html")));
    assert_eq!(requests_used(&connection, run_id), (site.request_count() + docs.request_count()) as i64);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g3_1_each_http_answer_keeps_its_own_state_and_redirects_are_not_followed() {
    let other = Fixture::start(routes(vec![("/landing", vec![response(200, &[], "样本公司")])]));
    let landing = other.url("/landing");
    let docs = Fixture::start(routes(vec![
        ("/moved", vec![response(302, &[("Location", landing.as_str())], "")]),
        ("/login", vec![response(401, &[], "")]),
        ("/forbidden", vec![response(403, &[], "")]),
        ("/gone", vec![response(404, &[], "")]),
        ("/busy", vec![response(429, &[], "")]),
        ("/broken", vec![response(500, &[], "")]),
        ("/spec.pdf", vec![response(200, &[("Content-Type", "application/pdf")], "%PDF-1.7\n样本公司 机密")]),
        ("/disguised.html", vec![response(200, &[("Content-Type", "text/html")], "PK\u{3}\u{4}样本公司")]),
        ("/gbk.html", vec![response(200, &[("Content-Type", "text/html; charset=gbk")], "<p>样本公司</p>")]),
    ]));
    let client = client(1_000_000);
    let cases: Vec<(&str, WorkItemStatus, SourceStatus, &str, &str)> = vec![
        ("/moved", WorkItemStatus::BlockedAuthorization, SourceStatus::Partial, "document_redirect_not_followed", "document_metadata"),
        ("/login", WorkItemStatus::Failed, SourceStatus::AuthFailed, "document_access_denied_401", "document_metadata"),
        ("/forbidden", WorkItemStatus::Failed, SourceStatus::AuthFailed, "document_access_denied_403", "document_metadata"),
        ("/gone", WorkItemStatus::Failed, SourceStatus::Failed, "document_not_found_404", "document_metadata"),
        ("/busy", WorkItemStatus::BlockedQuota, SourceStatus::RateLimited, "document_rate_limited_429", "document_metadata"),
        ("/broken", WorkItemStatus::Failed, SourceStatus::Failed, "document_http_500", "document_metadata"),
        ("/spec.pdf", WorkItemStatus::Completed, SourceStatus::Partial, "document_unsupported_type", "document_metadata"),
        ("/disguised.html", WorkItemStatus::Completed, SourceStatus::Partial, "document_unsupported_type", "document_metadata"),
        ("/gbk.html", WorkItemStatus::Failed, SourceStatus::ParseError, "document_charset_unsupported", "document_metadata"),
    ];
    let mut reasons = std::collections::BTreeSet::new();
    for (path, item_status, source_status, code, kind) in cases {
        let mut meter = meter(10);
        let outcome = read_link(&client, &docs.url(path), true, true, &mut meter);
        assert_eq!(outcome.status, Some(item_status), "{path}: {}", outcome.reason);
        assert_eq!(outcome.source_status, Some(source_status), "{path}");
        assert!(outcome.reason.starts_with(code), "{path}: {}", outcome.reason);
        assert!(outcome.findings.is_empty(), "{path} 不能产生线索");
        assert_eq!(outcome.observations.len(), 1, "{path}");
        let row = &outcome.observations[0];
        assert_eq!(row.kind, kind, "{path}");
        assert!(row.body.is_empty(), "{path} 的正文不能被当作已分析");
        assert_eq!(row.extra["contentAnalyzed"], false, "{path}");
        assert_eq!(meter.requests_used, 1, "{path} 只发一次");
        reasons.insert(code.to_string() + &format!("{source_status:?}"));
    }
    assert_eq!(reasons.len(), 8, "除 PDF 与伪装 zip 同为“类型不支持”外，各回答可辨");
    let moved = read_link(&client, &docs.url("/moved"), true, true, &mut meter(10));
    assert_eq!(moved.observations[0].extra["location"], landing.as_str());
    assert!(moved.reason.contains(&landing));
    assert_eq!(other.request_count(), 0, "跳转目标从不访问");
    let pdf = read_link(&client, &docs.url("/spec.pdf"), true, true, &mut meter(10));
    assert_eq!(pdf.observations[0].extra["unsupportedType"], "pdf");
    assert!(pdf.reason.contains("解析器未实现"), "{}", pdf.reason);
    let disguised = read_link(&client, &docs.url("/disguised.html"), true, true, &mut meter(10));
    assert_eq!(disguised.observations[0].extra["unsupportedType"], "zip_or_office", "魔数优先于声明类型");
}

#[test]
fn g3_1_private_ports_budget_cancel_and_bad_urls_are_refused_before_the_send() {
    let docs = Fixture::start(routes(vec![("/doc.txt", vec![response(200, &[("Content-Type", "text/plain")], "样本公司")])]));
    let client = client(1_000_000);
    let url = docs.url("/doc.txt");
    let mut used = meter(10);
    let private = read_link(&client, &url, false, true, &mut used);
    assert_eq!(private.status, Some(WorkItemStatus::BlockedAuthorization));
    assert!(private.reason.starts_with("document_private_denied"), "{}", private.reason);
    let port = read_link(&client, &url, true, false, &mut used);
    assert_eq!(port.status, Some(WorkItemStatus::BlockedAuthorization));
    assert!(port.reason.starts_with("document_port_denied"), "{}", port.reason);
    assert_eq!(used.requests_used, 0, "拒绝不扣额度");
    let budget = read_link(&client, &url, true, true, &mut meter(0));
    assert_eq!((budget.status, budget.source_status), (Some(WorkItemStatus::BlockedQuota), Some(SourceStatus::Partial)));
    assert!(budget.reason.starts_with("document_budget_exhausted"), "{}", budget.reason);
    let terms = terms();
    let target = entry(&url);
    let cancelled_context = LinkContext { client: &client, terms: &terms, allow_private: true, any_port: true, cancelled: &always };
    let cancelled = execute_link(&cancelled_context, &link_item(&target.url), &mut meter(10), Some(&target));
    assert_eq!(cancelled.status, Some(WorkItemStatus::Cancelled));
    let context = LinkContext { client: &client, terms: &terms, allow_private: true, any_port: true, cancelled: &never };
    let unconfigured = execute_link(&context, &link_item(UNCONFIGURED_ENTITY), &mut meter(10), None);
    assert_eq!((unconfigured.status, unconfigured.source_status), (Some(WorkItemStatus::SkippedUnconfigured), Some(SourceStatus::Unconfigured)));
    assert!(unconfigured.reason.starts_with("document_link_unconfigured"), "{}", unconfigured.reason);
    let missing = execute_link(&context, &link_item(&target.url), &mut meter(10), None);
    assert!(missing.reason.starts_with("document_link_not_in_frozen_list"), "{}", missing.reason);
    let userinfo = DocumentUrl { url: format!("http://user:pw@127.0.0.1:{}/doc.txt", docs.port), public_material: false, note: String::new() };
    let refused = execute_link(&context, &link_item(&userinfo.url), &mut meter(10), Some(&userinfo));
    assert!(refused.reason.starts_with("document_url_invalid"), "{}", refused.reason);
    assert_eq!(docs.request_count(), 0, "以上全部在发送前拒绝");
}

#[test]
fn g3_1_a_truncated_body_is_partial_and_never_claims_the_full_text() {
    let body = format!("样本公司 {}", "长文档内容。".repeat(40));
    let docs = Fixture::start(routes(vec![("/long.txt", vec![response(200, &[("Content-Type", "text/plain; charset=utf-8")], &body)])]));
    let outcome = read_link(&client(64), &docs.url("/long.txt"), true, true, &mut meter(10));
    assert_eq!((outcome.status, outcome.source_status), (Some(WorkItemStatus::Completed), Some(SourceStatus::Partial)));
    assert!(outcome.reason.starts_with("document_truncated"), "{}", outcome.reason);
    let row = &outcome.observations[0];
    assert!(row.truncated);
    assert_eq!(row.extra["contentAnalyzed"], true);
    assert_eq!(row.extra["mentions"], json!(["样本公司"]), "截断在多字节字符中间也只丢不完整的尾巴");
    assert!(row.extra.get("normalizedTextSha256").is_none(), "截断正文不能用于全文相同");
}

#[test]
fn g3_1_document_urls_are_validated_canonical_opt_in_and_keep_old_hashes() {
    let parsed = ExposureSettings::from_json(
        1,
        &json!({"exposureDocumentUrls": [
            "HTTPS://Docs.Example.TEST/a?v=1#frag",
            {"url": "http://example.test/b", "publicMaterial": true, "note": "国家标准公开文本"},
            "https://docs.example.test/a?v=1",
        ]}),
    )
    .unwrap();
    assert_eq!(
        parsed.document_urls,
        vec![
            DocumentUrl { url: "http://example.test/b".into(), public_material: true, note: "国家标准公开文本".into() },
            DocumentUrl { url: "https://docs.example.test/a?v=1".into(), public_material: false, note: String::new() },
        ]
    );
    let round_trip = ExposureSettings::from_json(1, &parsed.to_settings_json()).unwrap();
    assert_eq!(round_trip.document_urls, parsed.document_urls);
    let too_many: Vec<String> = (0..21).map(|index| format!("https://example.test/{index}")).collect();
    let rejected = [
        json!(["ftp://example.test/a"]),
        json!(["http://user:pw@example.test/a"]),
        json!(["not a url"]),
        json!([{"url": "https://example.test/a", "autoTrust": true}]),
        json!([{"url": "https://example.test/a", "publicMaterial": true}]),
        json!([{"url": "https://example.test/a", "note": "长".repeat(201)}]),
        json!([42]),
        json!("https://example.test/a"),
        json!(too_many),
    ];
    for value in rejected {
        let error = ExposureSettings::from_json(1, &json!({"exposureDocumentUrls": value.clone()})).expect_err(&value.to_string());
        assert!(error.starts_with("exposure_setting_invalid"), "{value}: {error}");
    }
    assert!(super::settings::SOURCES.contains(&"document_link"));
    assert!(!super::settings::DEFAULT_SOURCES.contains(&"document_link"), "默认不启用");
    let defaults = ExposureSettings::default().to_settings_json();
    assert!(defaults.get("exposureDocumentUrls").is_none(), "空列表不写进冻结配置，旧计划哈希不变");
}

#[test]
fn g3_1_enabled_without_urls_is_a_named_unconfigured_gap_with_no_request() {
    let site = site();
    let root = sandbox("nourl");
    let (connection, db_path) = database(&root);
    let pid = 312;
    let settings = document_project(&connection, pid, &site, &["passive_web", "document_link"], Vec::new());
    let run_id = run_round(&connection, &db_path, pid, &settings);
    let (status, error) = source_result(&connection, run_id, "document_link");
    assert_eq!(status, "unconfigured");
    assert!(error.starts_with("document_link_unconfigured"), "{error}");
    let item: String = connection
        .query_row("SELECT status FROM exposure_work_items WHERE run_id=?1 AND kind='document_link'", [run_id], |row| row.get(0))
        .unwrap();
    assert_eq!(item, "skipped_unconfigured");
    assert_eq!(requests_used(&connection, run_id), site.request_count() as i64);
    let _ = std::fs::remove_dir_all(root);
}

// ───────────────────────────── G3-2 intel ─────────────────────────────

fn intel_project(tag: &str, pid: i64) -> (PathBuf, Connection, JsonValue) {
    let root = sandbox(tag);
    let (connection, _db) = database(&root);
    project(&connection, pid, "http://127.0.0.1:9");
    let records = json!([
        {"title": "论坛帖子", "claim": "有人称出售样本公司客户数据", "source": "某论坛", "sourceUrl": "http://forum.invalid/t/1", "publishedAt": "2026-10-01"},
        {"title": "样本帖", "claim": "声称拿到样本公司员工通讯录", "sample": "姓名,邮箱\n张三,zhangsan@example.test", "source": "某频道"},
        {"title": "自称已核实", "claim": "样本公司 数据泄露", "sample": "无关的样本内容 abc", "grade": "human_verified"},
        {"title": "无关", "claim": "另一家公司数据出售"},
    ])
    .to_string();
    let csv = "title,claim,sample,source,entities\n频道转述,有人转卖数据,,某群,样本公司|example.test\n";
    let report = import_batch(
        &connection,
        pid,
        &import_request(&format!("op-intel-{tag}"), "public_intel_import", &[("feed.json", "json", records.as_bytes()), ("feed.csv", "csv", csv.as_bytes())]),
    )
    .expect("情报导入");
    (root, connection, report)
}

#[test]
fn g3_2_intel_records_are_claims_graded_by_their_own_samples() {
    let (root, connection, report) = intel_project("grades", 321);
    let pid = 321;
    let run_id = report["runId"].as_i64().unwrap();
    let json_url = file_url(&report, 0);
    let csv_url = file_url(&report, 1);
    let expect = [
        (format!("{json_url}/r1"), Some("claim_only"), false, ""),
        (format!("{json_url}/r2"), Some("sample_supported"), true, ""),
        (format!("{json_url}/r3"), Some("claim_only"), false, "导入方标注 human_verified，未经本系统人工复核"),
        (format!("{json_url}/r4"), None, false, ""),
        (format!("{csv_url}/r1"), Some("claim_only"), false, ""),
    ];
    for (url, grade, supported, note) in &expect {
        let (method, kind, _, _, detail) = observation(&connection, run_id, url);
        assert_eq!((method.as_str(), kind.as_str()), ("IMPORT", "intel_record"));
        assert_eq!(detail["sampleSupported"], *supported, "{url}");
        assert_eq!(detail["reportedSourceVerified"], false);
        assert!(detail["gradeNote"].as_str().unwrap().starts_with(note), "{url}: {detail}");
        match grade {
            Some(grade) => {
                assert_eq!(detail["evidenceGrade"], *grade, "{url}");
                let id = finding_id(&connection, pid, url, documents::TITLE_INTEL_CLAIM);
                assert_eq!(disposition(&connection, pid, url, documents::TITLE_INTEL_CLAIM), ("classification_pending".to_string(), "public_intel_import".to_string()));
                let category: String = connection
                    .query_row("SELECT info_category FROM exposure_classifications WHERE finding_id=?1", [id], |row| row.get(0))
                    .unwrap();
                assert_eq!(category, "sold_intel");
                let evidence: String = connection.query_row("SELECT evidence_excerpt FROM exposure_findings WHERE id=?1", [id], |row| row.get(0)).unwrap();
                assert!(evidence.contains(&format!("等级 {grade}")) && evidence.contains("声明，未验证"), "{evidence}");
            }
            None => assert_eq!(findings_at(&connection, pid, url), 0, "未出现集团名称的记录只留观察"),
        }
    }
    assert_eq!(report["files"][0]["result"]["findings"], 3);
    assert_eq!(report["files"][1]["result"]["findings"], 1);
    assert_eq!(source_result(&connection, run_id, "public_intel_import").0, "completed");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g3_2_a_manual_grade_needs_a_reason_and_a_verifiable_anchor_and_is_cas_guarded() {
    let (root, connection, report) = intel_project("manual", 322);
    let pid = 322;
    let run_id = report["runId"].as_i64().unwrap();
    let json_url = file_url(&report, 0);
    let r1 = finding_id(&connection, pid, &format!("{json_url}/r1"), documents::TITLE_INTEL_CLAIM);
    let r2 = finding_id(&connection, pid, &format!("{json_url}/r2"), documents::TITLE_INTEL_CLAIM);
    let obs = |suffix: &str| -> i64 {
        connection
            .query_row("SELECT id FROM exposure_observations WHERE run_id=?1 AND url=?2", params![run_id, format!("{json_url}/{suffix}")], |row| row.get(0))
            .unwrap()
    };
    let (o1, o2) = (obs("r1"), obs("r2"));
    let grade = |op: &str, finding: i64, expected: &str, to: &str, reason: &str, sample: Option<i64>, entity: Option<i64>| {
        grade_intel_finding(
            &connection,
            pid,
            &json!({"operationId": op, "findingId": finding, "expectedGrade": expected, "grade": to, "actor": "赵六", "reason": reason, "sampleObservationId": sample, "entityId": entity}),
        )
    };
    let refusals: Vec<(&str, Result<JsonValue, String>)> = vec![
        ("[exposure_intel_anchor_required]", grade("op-grade-0001", r1, "claim_only", "human_verified", "已与单位核实", None, None)),
        ("[exposure_intel_grade_invalid]", grade("op-grade-0002", r1, "claim_only", "human_verified", "短", None, Some(901))),
        ("[exposure_intel_anchor_required]", grade("op-grade-0003", r1, "claim_only", "human_verified", "样本不支持也想核实", Some(o1), None)),
        ("[exposure_intel_anchor_invalid]", grade("op-grade-0004", r1, "claim_only", "human_verified", "借用别的线索的样本", Some(o2), None)),
        ("[exposure_intel_anchor_invalid]", grade("op-grade-0005", r1, "claim_only", "human_verified", "目录里没有这个单位", None, Some(99_999))),
        ("[exposure_intel_anchor_required]", grade("op-grade-0006", r1, "claim_only", "sample_supported", "没有样本的样本支持", None, None)),
        ("[exposure_intel_grade_invalid]", grade("op-grade-0007", r1, "claim_only", "confirmed", "不存在的等级", None, None)),
    ];
    for (code, result) in refusals {
        let error = result.expect_err(code);
        assert!(error.starts_with(code), "{code}: {error}");
    }
    assert_eq!(count(&connection, "exposure_intel_grades"), 0, "拒绝必须零写");
    connection
        .execute(
            "INSERT INTO org_entities(id,group_id,parent_id,entity_kind,legal_name,normalized_name,region_code,directory_version) VALUES(901,NULL,NULL,'group','样本集团','样本集团-901','',1)",
            [],
        )
        .unwrap();
    let done = grade("op-grade-0010", r1, "claim_only", "human_verified", "与样本集团安全部电话核实", None, Some(901)).unwrap();
    assert_eq!((done["grade"].as_str(), done["previousGrade"].as_str(), done["replayed"].as_bool()), (Some("human_verified"), Some("claim_only"), Some(false)));
    let replay = grade("op-grade-0010", r1, "claim_only", "human_verified", "与样本集团安全部电话核实", None, Some(901)).unwrap();
    assert_eq!(replay["replayed"], true);
    assert_eq!(replay["eventId"], done["eventId"]);
    let conflict = grade("op-grade-0010", r1, "claim_only", "human_verified", "换了理由", None, Some(901)).unwrap_err();
    assert!(conflict.starts_with("[exposure_intel_grade_operation_conflict]"), "{conflict}");
    let stale = grade("op-grade-0011", r1, "claim_only", "claim_only", "按旧等级回退", None, None).unwrap_err();
    assert!(stale.starts_with("[exposure_intel_grade_conflict]"), "{stale}");
    let by_sample = grade("op-grade-0012", r2, "sample_supported", "human_verified", "样本中的邮箱已核对为在职员工", Some(o2), None).unwrap();
    assert_eq!(by_sample["grade"], "human_verified");
    assert_eq!(count(&connection, "exposure_intel_grades"), 2);
    // Another project cannot touch this finding, and nothing is written.
    project(&connection, 323, "http://127.0.0.1:10");
    let foreign = grade_intel_finding(
        &connection,
        323,
        &json!({"operationId": "op-grade-0020", "findingId": r1, "expectedGrade": "human_verified", "grade": "claim_only", "actor": "赵六", "reason": "跨项目尝试"}),
    )
    .unwrap_err();
    assert!(foreign.starts_with("[exposure_intel_not_found]"), "{foreign}");
    assert_eq!(count(&connection, "exposure_intel_grades"), 2);
    let evidence = super::repository::exposure_evidence(&connection, r1, pid).unwrap();
    assert_eq!(evidence["intelGrade"], "human_verified");
    let _ = std::fs::remove_dir_all(root);
}

// ───────────────────────────── G3-3 classification and same_document ─────────────────────────────

#[test]
fn g3_3_procurement_public_and_mixed_documents_reuse_the_g1_rules() {
    let site = site();
    let tender = "<html><title>样本公司门户运维项目招标公告</title><body>项目编号：ZB-01 采购人：样本公司 预算金额：100万 投标截止：2026-10-20</body></html>";
    let mixed = "<html><title>样本公司门户运维项目招标公告</title><body>项目编号：ZB-02 采购人：样本公司 预算金额：80万\npassword=Sup3rS3cretValue9</body></html>";
    let docs = Fixture::start(routes(vec![
        ("/tender.html", vec![html(tender)]),
        ("/tender-mixed.html", vec![html(mixed)]),
        ("/manual.html", vec![html("<html><title>产品手册</title><body>样本公司 云主机产品手册</body></html>")]),
        ("/manual-leak.html", vec![html("<html><title>产品手册</title><body>样本公司 部署说明\nsecret=Zq8LmN2pVx7RtY4w</body></html>")]),
        ("/plain.html", vec![html("<html><title>技术分享</title><body>样本公司 内网迁移经验</body></html>")]),
    ]));
    let root = sandbox("classify");
    let (connection, db_path) = database(&root);
    let pid = 331;
    let declared = |path: &str| DocumentUrl { url: docs.url(path), public_material: true, note: "公开产品手册".into() };
    let urls = vec![entry(&docs.url("/tender.html")), entry(&docs.url("/tender-mixed.html")), declared("/manual.html"), declared("/manual-leak.html"), entry(&docs.url("/plain.html"))];
    let settings = document_project(&connection, pid, &site, &["passive_web", "document_link"], urls);
    run_round(&connection, &db_path, pid, &settings);
    let d = |path: &str, title: &str| disposition(&connection, pid, &docs.url(path), title).0;
    assert_eq!(d("/tender.html", documents::TITLE_DOCUMENT_MENTION), "excluded_public_procurement");
    assert_eq!(d("/tender-mixed.html", documents::TITLE_DOCUMENT_CREDENTIAL), "classification_pending", "招采页里出现凭据：混合页留人工");
    assert_eq!(d("/manual.html", documents::TITLE_DOCUMENT_DECLARED_PUBLIC), "excluded_public_material");
    assert_eq!(d("/manual-leak.html", documents::TITLE_DOCUMENT_DECLARED_PUBLIC), "excluded_public_material");
    assert_eq!(d("/manual-leak.html", documents::TITLE_DOCUMENT_CREDENTIAL), "risk_candidate", "公开声明不能遮住凭据");
    assert_eq!(d("/plain.html", documents::TITLE_DOCUMENT_MENTION), "classification_pending");
    // The same notice imported as text goes through the same rules under its own channel.
    let text = "样本公司门户运维项目招标公告\n项目编号：ZB-01 采购人：样本公司 预算金额：100万 投标截止：2026-10-20";
    let report = import_batch(&connection, pid, &import_request("op-tender-0001", "document_import", &[("公告.txt", "text", text.as_bytes())])).unwrap();
    assert_eq!(
        disposition(&connection, pid, &file_url(&report, 0), documents::TITLE_DOCUMENT_MENTION),
        ("excluded_public_procurement".to_string(), "document_import".to_string())
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g3_3_same_document_is_a_candidate_only_for_identical_full_text_and_never_merges() {
    let site = site();
    let docs = Fixture::start(routes(vec![("/doc.html", vec![html("<html><title>t</title><body><p>样本公司 统一接口规范 第一章</p></body></html>")])]));
    let root = sandbox("same");
    let (connection, db_path) = database(&root);
    let pid = 332;
    let link = docs.url("/doc.html");
    let settings = document_project(&connection, pid, &site, &["passive_web", "document_link"], vec![entry(&link)]);
    let link_run = run_round(&connection, &db_path, pid, &settings);
    let link_obs: i64 = connection
        .query_row("SELECT id FROM exposure_observations WHERE run_id=?1 AND url=?2", params![link_run, link], |row| row.get(0))
        .unwrap();
    let same = import_batch(&connection, pid, &import_request("op-same-00001", "document_import", &[("规范.txt", "text", "t\n样本公司  统一接口规范\n第一章".as_bytes())])).unwrap();
    let (_, _, _, _, detail) = observation(&connection, same["runId"].as_i64().unwrap(), &file_url(&same, 0));
    let candidates = detail["sameDocumentCandidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 1, "{detail}");
    assert_eq!(candidates[0]["observationId"], link_obs);
    assert_eq!(candidates[0]["url"], link.as_str());
    assert_eq!(candidates[0]["relation"], "same_document");
    assert_eq!(candidates[0]["status"], "candidate");
    let renamed = import_batch(&connection, pid, &import_request("op-same-00002", "document_import", &[("规范.txt", "text", "t 样本公司 统一接口规范 第二章".as_bytes())])).unwrap();
    let (_, _, _, _, other) = observation(&connection, renamed["runId"].as_i64().unwrap(), &file_url(&renamed, 0));
    assert_eq!(other["sameDocumentCandidates"], json!([]), "同名不同内容不是同一文档");
    // Candidates never merge leads: each location keeps its own finding.
    assert_eq!(findings_at(&connection, pid, &link), 1);
    assert_eq!(findings_at(&connection, pid, &file_url(&same, 0)), 1);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g3_3_new_channels_map_apart_and_intel_never_becomes_risk_by_itself() {
    use super::classify::{classify, page_context, source_channel};
    use super::observations::FindingDraft;
    assert_eq!(source_channel("document_link"), "document_link");
    assert_eq!(source_channel("document_import"), "document_import");
    assert_eq!(source_channel("public_intel_import"), "public_intel_import");
    let draft = |category: &str, title: &str, confidence: i64| FindingDraft {
        category: category.into(),
        title: title.into(),
        url: "import://op/abc/x".into(),
        source_type: "x".into(),
        evidence: String::new(),
        severity: "high".into(),
        confidence,
        asset_id: None,
        content_hash: String::new(),
    };
    let context = page_context("import://op/abc/x", b"plain text", "text/plain");
    let credential = draft("credential", documents::TITLE_DOCUMENT_CREDENTIAL, 65);
    assert_eq!(classify(&credential, "public_intel_import", &context).disposition, "classification_pending");
    assert_eq!(classify(&credential, "document_import", &context).disposition, "risk_candidate");
    assert_eq!(classify(&credential, "document_link", &context).disposition, "risk_candidate");
    let mention = draft("document", documents::TITLE_DOCUMENT_MENTION, 30);
    assert_eq!(classify(&mention, "document_link", &context).disposition, "classification_pending");
    assert_eq!(classify(&mention, "document_import", &context).disposition, "classification_pending");
    let declared = draft("document", documents::TITLE_DOCUMENT_DECLARED_PUBLIC, 30);
    assert_eq!(classify(&declared, "document_import", &context).disposition, "excluded_public_material");
    let intel = draft("intel_claim", documents::TITLE_INTEL_CLAIM, 40);
    let verdict = classify(&intel, "public_intel_import", &context);
    assert_eq!((verdict.disposition, verdict.info_category), ("classification_pending", "sold_intel"));
}

// ───────────────────────────── G3-4 closed loop and settings view ─────────────────────────────

#[test]
fn g3_4_a_reviewed_document_lead_survives_a_rerun_and_changed_content_asks_again() {
    use super::remediation::{review_scoped, top_revision};
    let site = site();
    let v1 = html("<html><title>规范</title><body>样本公司 接口规范 v1</body></html>");
    let v2 = html("<html><title>规范</title><body>样本公司 接口规范 v2（新增内网地址段）</body></html>");
    let docs = Fixture::start(routes(vec![("/loop.html", vec![v1.clone(), v1, v2])]));
    let root = sandbox("loop");
    let (connection, db_path) = database(&root);
    let pid = 341;
    let url = docs.url("/loop.html");
    let settings = document_project(&connection, pid, &site, &["passive_web", "document_link"], vec![entry(&url)]);
    let r1 = run_round(&connection, &db_path, pid, &settings);
    let id = finding_id(&connection, pid, &url, documents::TITLE_DOCUMENT_MENTION);
    let obs: i64 = connection.query_row("SELECT id FROM exposure_observations WHERE run_id=?1 AND url=?2", params![r1, url], |row| row.get(0)).unwrap();
    review_scoped(&connection, pid, id, top_revision(&connection, id).unwrap(), "dismissed", "李四", "公开对接文档", &[obs]).expect("复核写入");
    let needs_review = || -> i64 {
        connection
            .query_row("SELECT COALESCE(MAX(review_needed),0) FROM exposure_finding_versions WHERE finding_id=?1", [id], |row| row.get(0))
            .unwrap()
    };
    run_round(&connection, &db_path, pid, &settings);
    assert_eq!(findings_at(&connection, pid, &url), 1, "跨轮同一文档只有一条线索");
    let status: String = connection.query_row("SELECT status FROM exposure_findings WHERE id=?1", [id], |row| row.get(0)).unwrap();
    assert_eq!(status, "dismissed");
    assert_eq!(needs_review(), 0);
    run_round(&connection, &db_path, pid, &settings);
    assert_eq!(needs_review(), 1, "内容变化必须要求重新复核");
    let status: String = connection.query_row("SELECT status FROM exposure_findings WHERE id=?1", [id], |row| row.get(0)).unwrap();
    assert_eq!(status, "dismissed", "旧结论保留，不被冒用也不被删除");
    assert_eq!(docs.request_count(), 3);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g3_4_the_settings_view_lists_document_capabilities_and_named_unsupported_directions() {
    let root = sandbox("view");
    let (connection, _db) = database(&root);
    let pid = 342;
    let site = site();
    document_project(&connection, pid, &site, &["passive_web"], Vec::new());
    let state_of = |view: &JsonValue, capability: &str| -> (String, String) {
        let row = view["sourceCapabilities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["capability"] == capability)
            .unwrap_or_else(|| panic!("缺少能力 {capability}"))
            .clone();
        (row["state"].as_str().unwrap().to_string(), row["reason"].as_str().unwrap_or_default().to_string())
    };
    let view = super::exposure_settings_view_in(&connection, pid).unwrap();
    assert_eq!(state_of(&view, "explicit_document_url").0, "unconfigured");
    let (import_state, import_reason) = state_of(&view, "file_import");
    assert_eq!(import_state, "configured");
    assert!(import_reason.contains("已注册"), "{import_reason}");
    assert!(!import_reason.contains("尚未注册"), "{import_reason}");
    let (intel_state, intel_reason) = state_of(&view, "public_intel_import");
    assert_eq!(intel_state, "configured");
    assert!(intel_reason.contains("已注册"), "{intel_reason}");
    for name in ["pdf_docx_ocr", "netdisk_keyword_search", "underground_feed"] {
        assert_eq!(state_of(&view, name).0, "unsupported", "{name}");
    }
    assert_eq!(state_of(&view, "repository_metadata").0, "configured", "GitHub 能力仍在");
    assert!(view["bounds"]["sources"].as_array().unwrap().iter().any(|row| row == "document_link"));
    assert!(!view["defaultSources"].as_array().unwrap().iter().any(|row| row == "document_link"));
    document_project(&connection, pid, &site, &["passive_web", "document_link"], vec![entry("https://docs.example.test/a")]);
    let view = super::exposure_settings_view_in(&connection, pid).unwrap();
    let (state, reason) = state_of(&view, "explicit_document_url");
    assert_eq!(state, "configured");
    assert!(reason.contains("1 个地址"), "{reason}");
    let _ = std::fs::remove_dir_all(root);
}

/// Bounded live check: read two small public, non-sensitive documents (an RFC that reserves
/// the example domains, and the IANA example page) once each with the production gates
/// (no private egress, default ports only, no redirect). Nothing is written anywhere.
/// Run explicitly: `cargo test --lib g3_live -- --ignored --nocapture`.
#[test]
#[ignore]
fn g3_live_public_documents_are_read_once_each() {
    let client = client(1_000_000);
    let terms = vec!["example.com".to_string()];
    for url in ["https://www.rfc-editor.org/rfc/rfc2606.txt", "https://example.com/"] {
        let mut meter = meter(1);
        let context = LinkContext { client: &client, terms: &terms, allow_private: false, any_port: false, cancelled: &never };
        let target = entry(url);
        let started = chrono::Local::now();
        let outcome = execute_link(&context, &link_item(&target.url), &mut meter, Some(&target));
        let row = outcome.observations.first();
        println!(
            "LIVE {} at {} → item={:?} source={:?} reason={} requests={} status={:?} type={:?} bytes={:?} mentions={:?} findings={}",
            url,
            started.format("%Y-%m-%d %H:%M:%S %z"),
            outcome.status,
            outcome.source_status,
            outcome.reason,
            meter.requests_used,
            row.map(|r| r.status_code),
            row.map(|r| r.content_type.clone()),
            row.map(|r| r.body_bytes),
            row.map(|r| r.extra["mentions"].clone()),
            outcome.findings.len()
        );
        assert_eq!(meter.requests_used, 1);
        assert!(row.is_some());
    }
}

// ───────────────────────────── P1 修复回归（2026-10-09） ─────────────────────────────

/// P1-5：导入时的来源声明、批次名、公开材料说明，以及人工改等级的理由，贴进来的秘密不落库。
#[test]
fn rem_p1_5_secrets_typed_into_import_declarations_and_grades_never_reach_the_database() {
    let secret = "Kp4Hunter2SecretValue77";
    let token = "ghp_RemImportToken0123456789abcdefXYZ";
    let root = sandbox("rem-import-secret");
    let (connection, _db) = database(&root);
    let pid = 391;
    project(&connection, pid, "http://127.0.0.1:9");
    let mut request = import_request("op-rem-import-01", "document_import", &[("周报.txt", "text", "样本公司 内部周报".as_bytes())]);
    request["sourceDeclaration"] = json!(format!("同事转来的，压缩包 password={secret}"));
    request["batchLabel"] = json!(format!("批次 Authorization: Bearer {token}"));
    request["publicMaterial"] = json!(true);
    request["publicMaterialNote"] = json!(format!("公开网盘 https://u:{secret}@pan.example.test/s/1"));
    let first = import_batch(&connection, pid, &request).expect("导入成功");
    let again = import_batch(&connection, pid, &request).expect("同一载荷重放");
    assert_eq!(again["replayed"], true, "脱敏不破坏幂等");
    assert_eq!(again["runId"], first["runId"]);
    let (root2, connection2, report) = intel_project("rem-grade-secret", 392);
    let json_url = file_url(&report, 0);
    let r1 = finding_id(&connection2, 392, &format!("{json_url}/r1"), documents::TITLE_INTEL_CLAIM);
    connection2
        .execute(
            "INSERT INTO org_entities(id,group_id,parent_id,entity_kind,legal_name,normalized_name,region_code,directory_version) VALUES(901,NULL,NULL,'group','样本集团','样本集团-901','',1)",
            [],
        )
        .unwrap();
    grade_intel_finding(
        &connection2,
        392,
        &json!({"operationId": "op-rem-grade-01", "findingId": r1, "expectedGrade": "claim_only", "grade": "human_verified", "actor": "赵六", "reason": format!("对方给了后台 password={secret} 核实"), "entityId": 901}),
    )
    .expect("改等级成功");
    for (label, stored) in [("import", everything_stored(&connection)), ("grade", everything_stored(&connection2))] {
        assert!(!stored.contains(secret), "{label}：口令原文落库了");
        assert!(!stored.contains(token), "{label}：token 原文落库了");
    }
    assert!(everything_stored(&connection2).contains("Kp4***"), "脱敏后的形态仍可对账");
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(root2);
}

/// FIX3-R6：导入批次的来源声明里粘了秘密；两段秘密脱敏后相同，同一 operationId 不能重放旧批次。
#[test]
fn fix3_r6_an_import_with_a_colliding_redacted_declaration_conflicts_instead_of_replaying() {
    let root = sandbox("fix3-r6-import");
    let (connection, _db) = database(&root);
    let pid = 306;
    project(&connection, pid, "http://127.0.0.1:9");
    let with = |secret: &str| {
        let mut request = import_request("op-f3r6-imp1", "document_import", &[("a.txt", "text", "样本公司 资料".as_bytes())]);
        request["sourceDeclaration"] = json!(format!("内部共享盘 token={secret} 导出"));
        request
    };
    let first = import_batch(&connection, pid, &with("SENTINELa1b2c3d4ZZ")).unwrap();
    let after = footprint(&connection);
    assert_eq!(import_batch(&connection, pid, &with("SENTINELa1b2c3d4ZZ")).unwrap()["replayed"], true);
    let error = import_batch(&connection, pid, &with("SENTINELx9y8w7v6ZZ")).unwrap_err();
    assert!(error.starts_with("[exposure_import_operation_conflict]"), "{error}");
    assert_eq!(footprint(&connection), after);
    assert!(!first.to_string().contains("SENTINELa1b2c3d4ZZ"));
    let _ = std::fs::remove_dir_all(root);
}
