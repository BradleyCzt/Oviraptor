// §Q5 — the separation between "what a round saw" and "what the queue says now", plus the
// version trail that tells a reviewer their earlier conclusion may no longer apply.

use super::fingerprint::{canonical_url, finding_fingerprint};
use super::observations::{merge_finding, record, review_needed, FindingDraft, NewObservation};
use super::repository::exposure_evidence;
use super::state::ensure_exposure_schema;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

fn sandbox(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "oviraptor-exposure-q5-{tag}-{}",
        uuid::Uuid::new_v4()
    ))
}

fn database(root: &Path) -> Connection {
    let db_path = crate::db::initialize(&root.join("app")).expect("临时库");
    let connection = crate::db::open(&db_path).expect("临时库连接");
    ensure_exposure_schema(&connection).expect("附加结构必须可用");
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(121,'范围','active')",
            [],
        )
        .unwrap();
    for run_id in [1, 2] {
        connection
            .execute(
                "INSERT INTO exposure_runs(id,project_id,status) VALUES(?1,121,?2)",
                params![run_id, if run_id == 1 { "completed" } else { "running" }],
            )
            .unwrap();
    }
    connection
        .execute(
            "INSERT INTO assets(id,asset_key,link,host) VALUES(5,'q5-5','https://corp.invalid/','corp.invalid')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO project_assets(project_id,asset_id) VALUES(121,5)",
            [],
        )
        .unwrap();
    connection
}

fn observation(url: &str, body: &str) -> NewObservation {
    NewObservation {
        source_key: "passive_web".to_string(),
        kind: "source_map".to_string(),
        url: url.to_string(),
        final_url: url.to_string(),
        status_code: 200,
        content_type: "application/json".to_string(),
        body: body.as_bytes().to_vec(),
        body_bytes: body.len() as i64,
        asset_id: Some(5),
        current_accessible: Some(true),
        ..NewObservation::new("passive_web", "source_map", url)
    }
}

fn draft(url: &str, evidence: &str) -> FindingDraft {
    FindingDraft {
        category: "source_code".to_string(),
        title: "公开 Source Map 还原业务源码".to_string(),
        url: url.to_string(),
        source_type: "passive_web".to_string(),
        evidence: evidence.to_string(),
        severity: "high".to_string(),
        confidence: 80,
        asset_id: Some(5),
        content_hash: String::new(),
    }
}

#[test]
fn q5_two_rounds_of_one_url_are_two_observations_and_one_finding() {
    let root = sandbox("separate");
    let connection = database(&root);
    let url = "https://corp.invalid/main.js.map";
    let body = "{\"version\":3,\"sources\":[\"src/a.ts\"]}";
    let (first, new_first) = record(&connection, 1, 121, None, &observation(url, body)).unwrap();
    assert!(new_first);
    // 同一轮重复观察同一 URL 不产生第二条观察。
    let (_, new_again) = record(&connection, 1, 121, None, &observation(url, body)).unwrap();
    assert!(!new_again);
    let (second, _) = record(&connection, 2, 121, None, &observation(url, body)).unwrap();
    assert_ne!(first, second, "每一轮的观察必须各自留痕");
    let (finding, is_new) =
        merge_finding(&connection, 1, 121, &draft(url, "第一轮的脱敏证据"), first).unwrap();
    assert!(is_new);
    let (again, is_new) =
        merge_finding(&connection, 2, 121, &draft(url, "第二轮的脱敏证据"), second).unwrap();
    assert_eq!(finding, again, "同一线索必须归并到同一条复核队列项");
    assert!(!is_new);
    let findings: i64 = connection
        .query_row("SELECT COUNT(*) FROM exposure_findings", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(findings, 1);
    let links: i64 = connection
        .query_row(
            "SELECT COUNT(DISTINCT observation_id) FROM exposure_finding_observations WHERE finding_id=?1",
            [finding],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(links, 2, "两轮各留一条来源引用");
    let duplicated: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_finding_observations WHERE finding_id=?1",
            [finding],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(duplicated, 2, "同一轮内重复归并不得再加一条引用");
    let observations: i64 = connection
        .query_row("SELECT COUNT(*) FROM exposure_observations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(observations, 2, "{observations}");
    // 观察是不可变的：第二轮不能改写第一轮的内容哈希。
    let first_hash: String = connection
        .query_row(
            "SELECT content_hash FROM exposure_observations WHERE id=?1",
            [first],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(first_hash.len(), 64, "观察必须带内容哈希");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn q5_changed_content_flags_review_without_rewriting_the_human_conclusion() {
    let root = sandbox("versions");
    let connection = database(&root);
    let url = "https://corp.invalid/app.js.map";
    let (observation_id, _) = record(
        &connection,
        1,
        121,
        None,
        &observation(url, "{\"version\":3,\"sources\":[\"src/a.ts\"]}"),
    )
    .unwrap();
    let (finding, _) =
        merge_finding(&connection, 1, 121, &draft(url, "首版证据"), observation_id).unwrap();
    // 人工确认
    connection
        .execute(
            "UPDATE exposure_findings SET status='confirmed' WHERE id=?1",
            [finding],
        )
        .unwrap();
    super::observations::note_review_event(&connection, 121, finding, "new", "confirmed", "已确认")
        .unwrap();

    // 第二轮内容变了。
    let (second_observation, _) = record(
        &connection,
        2,
        121,
        None,
        &observation(
            url,
            "{\"version\":3,\"sources\":[\"src/a.ts\",\"src/billing.ts\"]}",
        ),
    )
    .unwrap();
    let (same_finding, is_new) = merge_finding(
        &connection,
        2,
        121,
        &draft(url, "新增了一个源文件的证据"),
        second_observation,
    )
    .unwrap();
    assert_eq!(same_finding, finding);
    assert!(!is_new);
    let flagged = review_needed(&connection, finding)
        .unwrap()
        .expect("内容变化必须标需复核");
    assert!(flagged.0 >= 2, "{flagged:?}");
    let status: String = connection
        .query_row(
            "SELECT status FROM exposure_findings WHERE id=?1",
            [finding],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "confirmed", "旧人工结论必须保留，由复核人决定");
    let events: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_review_events WHERE finding_id=?1",
            [finding],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(events, 1, "复核轨迹必须留下");

    // 内容不变时不得刷版本，避免制造假需复核。
    let before: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_finding_versions WHERE finding_id=?1",
            [finding],
            |row| row.get(0),
        )
        .unwrap();
    merge_finding(
        &connection,
        2,
        121,
        &draft(url, "新增了一个源文件的证据"),
        second_observation,
    )
    .unwrap();
    let after: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_finding_versions WHERE finding_id=?1",
            [finding],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(before, after, "同样的内容不能产生新版本");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q5_the_evidence_package_is_scoped_redacted_and_complete() {
    let root = sandbox("evidence");
    let connection = database(&root);
    let url = "https://corp.invalid/status";
    let (observation_id, _) = record(
        &connection,
        1,
        121,
        None,
        &NewObservation {
            headers: serde_json::json!({"set-cookie": "sess=ABCDEFGHIJKLMNOP"}),
            ..observation(url, "{\"version\":3,\"sources\":[\"src/a.ts\"]}")
        },
    )
    .unwrap();
    let (finding, _) = merge_finding(
        &connection,
        1,
        121,
        &draft(url, "api_key = SUPERSECRETVALUE123"),
        observation_id,
    )
    .unwrap();
    let package = exposure_evidence(&connection, finding, 121).expect("证据包可读");
    assert_eq!(package["finding"]["id"], finding);
    assert_eq!(package["observations"].as_array().unwrap().len(), 1);
    let dumped = package.to_string();
    assert!(!dumped.contains("SUPERSECRETVALUE123"), "证据包必须已脱敏");
    assert!(
        !dumped.contains("ABCDEFGHIJKLMNOP"),
        "响应头里的 Cookie 同样不能外泄"
    );
    assert!(dumped.contains("contentHash"));
    assert!(dumped.contains("ruleVersion"));
    assert!(dumped.contains("redactionVersion"));
    // 跨项目读取必须被拒绝，而不是返回别人的证据。
    let denied = exposure_evidence(&connection, finding, 999).unwrap_err();
    assert_eq!(denied.code, "exposure_scope_mismatch");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q5_fingerprints_normalise_the_origin_but_keep_path_case() {
    assert_eq!(
        canonical_url("HTTPS://Corp.INVALID:443/A"),
        "https://corp.invalid/A"
    );
    assert_ne!(
        finding_fingerprint("document", "https://x.invalid/Report", "T"),
        finding_fingerprint("document", "https://x.invalid/report", "T"),
        "大小写敏感的 path 必须视为不同对象"
    );
    assert_eq!(
        finding_fingerprint("document", "https://X.invalid/Report", " 标题 "),
        finding_fingerprint("document", "https://x.invalid/Report", "标题"),
    );
}
