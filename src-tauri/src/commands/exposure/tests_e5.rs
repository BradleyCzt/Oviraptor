// §E5 — review, remediation and retest. The assertions are about what a review may NOT do:
// overwrite somebody else's conclusion, silently re-interpret an old observation, or turn "the
// retest did not fetch it" into "the exposure is gone".

use super::fixture::seed_scope;
use super::observations::{merge_finding, record, FindingDraft, NewObservation};
use super::orchestrator::{run_plan, start_or_reuse, ItemOutcome, KIND_PASSIVE_WEB};
use super::remediation::{
    mark_remediation, page_remediations, record_remediation, retest_outcome, review_scoped,
    review_trail, start_retest, taxonomy, top_revision,
};
use super::settings::{save, ExposureSettings};
use super::state::{ensure_exposure_schema, SourceStatus, WorkItemStatus};
use rusqlite::{params, Connection};
use serde_json::{json, Value as JsonValue};
use std::path::{Path, PathBuf};

fn sandbox(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "oviraptor-exposure-e5-{tag}-{}",
        uuid::Uuid::new_v4()
    ))
}

fn database(root: &Path) -> Connection {
    let db_path = crate::db::initialize(&root.join("app")).expect("临时库");
    let connection = crate::db::open(&db_path).expect("连接");
    ensure_exposure_schema(&connection).expect("附加结构必须可用");
    connection
        .pragma_update(None, "synchronous", "OFF")
        .expect("测试库 pragma");
    connection
}

fn project(connection: &Connection, id: i64, name: &str) {
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(?1,?2,'active')",
            params![id, name],
        )
        .unwrap();
}

fn run(connection: &Connection, id: i64, project_id: i64, status: &str) {
    connection
        .execute(
            "INSERT INTO exposure_runs(id,project_id,status,stage) VALUES(?1,?2,?3,'planned')",
            params![id, project_id, status],
        )
        .unwrap();
}

/// One observation plus the finding it produced; returns (finding, observation).
fn observed(
    connection: &Connection,
    run_id: i64,
    project_id: i64,
    url: &str,
    body: &str,
    title: &str,
) -> (i64, i64) {
    let (observation_id, _) = record(
        connection,
        run_id,
        project_id,
        None,
        &NewObservation {
            body: body.as_bytes().to_vec(),
            body_bytes: body.len() as i64,
            status_code: 200,
            ..NewObservation::new(KIND_PASSIVE_WEB, "resource", url)
        },
    )
    .expect("观察写入");
    let (finding_id, _) = merge_finding(
        connection,
        run_id,
        project_id,
        &FindingDraft {
            category: "source_code".to_string(),
            title: title.to_string(),
            url: url.to_string(),
            source_type: KIND_PASSIVE_WEB.to_string(),
            evidence: "internal/repo/.env 片段".to_string(),
            severity: "high".to_string(),
            confidence: 75,
            asset_id: None,
            content_hash: String::new(),
        },
        observation_id,
    )
    .expect("线索归并");
    (finding_id, observation_id)
}

/// Runs a round to its terminal state with no further fetches, so the next start is a new run.
fn drain(connection: &Connection, run_id: i64, owner: &str) {
    let mut executor = |_item: &super::work_items::WorkItem| -> Result<ItemOutcome, String> {
        Ok(ItemOutcome {
            status: Some(WorkItemStatus::Completed),
            source_status: Some(SourceStatus::Completed),
            ..Default::default()
        })
    };
    run_plan(connection, run_id, owner, &mut executor).expect("本轮收敛");
}

fn current_settings(connection: &Connection, project_id: i64) -> JsonValue {
    let mut value = ExposureSettings::for_project(project_id);
    value.sources = vec![KIND_PASSIVE_WEB.to_string()];
    save(connection, &value, 0).expect("配置必须能保存");
    super::exposure_scan_settings(connection, project_id).expect("配置必须可读")
}

#[test]
fn e5_a_review_must_carry_the_revision_it_read_and_leaves_history_intact() {
    let root = sandbox("review");
    let connection = database(&root);
    project(&connection, 191, "范围");
    run(&connection, 19100, 191, "completed");
    let (finding, observation) = observed(
        &connection,
        19100,
        191,
        "https://r.e5.invalid/.env",
        "SECRETVALUE=first",
        "公开仓库里的凭据片段",
    );
    let top = top_revision(&connection, finding).expect("版本");
    assert!(top >= 1, "归并必须留下版本行：r{top}");

    // No reason, no actor, unknown status, stale revision, wrong project: all refused.
    assert!(
        review_scoped(&connection, 191, finding, top, "confirmed", "王凡", "", &[])
            .expect_err("没有依据的结论不能写入")
            .contains("exposure_review_reason_required")
    );
    assert!(review_scoped(
        &connection,
        191,
        finding,
        top,
        "confirmed",
        "",
        "看过证据",
        &[]
    )
    .expect_err("匿名结论不能进队列")
    .contains("exposure_review_actor_invalid"));
    assert!(review_scoped(
        &connection,
        191,
        finding,
        top,
        "fixed",
        "王凡",
        "看过证据",
        &[]
    )
    .expect_err("未知状态必须拒绝")
    .contains("exposure_review_status_invalid"));
    assert!(review_scoped(
        &connection,
        192,
        finding,
        top,
        "confirmed",
        "王凡",
        "看过证据",
        &[]
    )
    .expect_err("跨项目必须拒绝")
    .contains("exposure_scope_mismatch"));
    let status: String = connection
        .query_row(
            "SELECT status FROM exposure_findings WHERE id=?1",
            [finding],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "new", "被拒绝的复核不能留下半个结论");

    // A queue that moved on is refused with the real numbers.
    assert!(review_scoped(
        &connection,
        191,
        finding,
        999,
        "confirmed",
        "王凡",
        "看过证据",
        &[]
    )
    .expect_err("版本不符必须拒绝")
    .contains("exposure_review_conflict"));

    assert_eq!(
        review_scoped(
            &connection,
            191,
            finding,
            top,
            "confirmed",
            "王凡",
            "两处字节日志都指向同一份 .env",
            &[observation],
        )
        .expect("复核必须能写入"),
        top
    );
    let after: String = connection
        .query_row(
            "SELECT status FROM exposure_findings WHERE id=?1",
            [finding],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(after, "confirmed");
    let trail = review_trail(&connection, 191, finding).expect("轨迹");
    assert_eq!(trail.len(), 1, "{trail:?}");
    assert_eq!(trail[0]["actor"], json!("王凡"));
    assert_eq!(trail[0]["fromStatus"], json!("new"));
    assert_eq!(trail[0]["toStatus"], json!("confirmed"));
    assert_eq!(trail[0]["expectedRevision"], json!(top));
    assert_eq!(
        trail[0]["references"],
        json!([observation]),
        "复核要能指回证据"
    );
    assert!(trail[0]["createdAt"].as_str().unwrap_or_default().len() >= 19);

    // The second round's new bytes version the finding without erasing round one's evidence.
    run(&connection, 19101, 191, "completed");
    let (_, second_observation) = observed(
        &connection,
        19101,
        191,
        "https://r.e5.invalid/.env",
        "SECRETVALUE=changed",
        "公开仓库里的凭据片段",
    );
    let observations: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_observations WHERE project_id=191",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(observations, 2, "复核与复测都不能删旧观察");
    let first_hash: String = connection
        .query_row(
            "SELECT content_hash FROM exposure_observations WHERE id=?1",
            [observation],
            |row| row.get(0),
        )
        .unwrap();
    let second_hash: String = connection
        .query_row(
            "SELECT content_hash FROM exposure_observations WHERE id=?1",
            [second_observation],
            |row| row.get(0),
        )
        .unwrap();
    assert_ne!(first_hash, second_hash, "内容变化必须留下两份不同的字节");
    assert_ne!(second_observation, observation);
}

#[test]
fn e5_a_remediation_is_bound_to_the_finding_observation_and_round() {
    let root = sandbox("remediation");
    let connection = database(&root);
    project(&connection, 192, "范围");
    run(&connection, 19200, 192, "completed");
    let (finding, observation) = observed(
        &connection,
        19200,
        192,
        "https://m.e5.invalid/config",
        "config bytes",
        "公开配置",
    );
    let id = record_remediation(
        &connection,
        192,
        finding,
        Some(observation),
        19200,
        "蓝队-李某",
        "已从镜像移除该文件",
        "fixing",
    )
    .expect("整改登记");
    assert!(id > 0);
    // Same evidence again updates the row instead of duplicating it.
    let again = record_remediation(
        &connection,
        192,
        finding,
        Some(observation),
        19200,
        "蓝队-李某",
        "补充：也清了 CDN 缓存",
        "fixing",
    )
    .expect("重复登记");
    assert_eq!(again, id, "同一证据只该有一条整改记录");
    let rows = page_remediations(&connection, 192, "", "", 10).expect("分页");
    assert_eq!(rows.total, 1, "{rows:?}");
    assert_eq!(rows.items[0].note, "补充：也清了 CDN 缓存");
    assert_eq!(rows.items[0].run_id, 19200, "整改要记住来源轮次");

    // Refusals: foreign finding, missing evidence, anonymous, unknown status.
    assert!(record_remediation(
        &connection,
        193,
        finding,
        Some(observation),
        19200,
        "李某",
        "x",
        "fixing"
    )
    .expect_err("别的项目的线索不能挂整改")
    .contains("exposure_scope_mismatch"));
    assert!(record_remediation(
        &connection,
        192,
        finding,
        Some(55555),
        19200,
        "李某",
        "x",
        "fixing"
    )
    .expect_err("不存在的观察不能当依据")
    .contains("exposure_remediation_evidence_missing"));
    assert!(record_remediation(
        &connection,
        192,
        finding,
        Some(observation),
        19200,
        "",
        "x",
        "fixing"
    )
    .expect_err("匿名整改必须拒绝")
    .contains("exposure_remediation_incomplete"));
    assert!(record_remediation(
        &connection,
        192,
        finding,
        Some(observation),
        19200,
        "李某",
        "x",
        "done"
    )
    .expect_err("未知状态必须拒绝")
    .contains("exposure_remediation_status_invalid"));
    // Paging guards behave like the rest of the surface.
    let bad = page_remediations(&connection, 192, "done", "", 10)
        .expect_err("未知整改状态不能当成无筛选");
    assert_eq!(bad.code, "exposure_remediation_status_invalid");
    let huge = page_remediations(&connection, 192, "", "", 500).expect_err("页长超界必须拒绝");
    assert_eq!(huge.code, "exposure_page_size_invalid");
    let other = page_remediations(&connection, 194, "", "", 10).expect("其他项目可读");
    assert_eq!(other.total, 0);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn e5_a_retest_starts_a_new_round_and_only_changed_bytes_may_close() {
    let root = sandbox("retest");
    let connection = database(&root);
    project(&connection, 195, "范围");
    seed_scope(&connection, 195, "http://t.e5.invalid/", true);
    let settings = current_settings(&connection, 195);
    let first = start_or_reuse(&connection, 195, &settings).expect("第一轮");
    let url = "http://t.e5.invalid/.env";
    let (finding, observation) = observed(
        &connection,
        first.run_id,
        195,
        url,
        "leaked-bytes",
        "泄露的 .env",
    );
    run_plan(
        &connection,
        first.run_id,
        "owner-e5-a",
        &mut |_item| -> Result<ItemOutcome, String> {
            Ok(ItemOutcome {
                status: Some(WorkItemStatus::Completed),
                source_status: Some(SourceStatus::Completed),
                ..Default::default()
            })
        },
    )
    .expect("第一轮收敛");
    let remediation = record_remediation(
        &connection,
        195,
        finding,
        Some(observation),
        first.run_id,
        "蓝队",
        "已下线该文件",
        "fixing",
    )
    .expect("整改登记");
    let before_hash: String = connection
        .query_row(
            "SELECT content_hash FROM exposure_observations WHERE id=?1",
            [observation],
            |row| row.get(0),
        )
        .unwrap();

    // No retest round yet: the report says so and closing is refused.
    let outcome = retest_outcome(&connection, 195, remediation).expect("只读比对");
    assert_eq!(outcome["basis"], json!("no_retest_round"));
    assert_eq!(outcome["canClose"], json!(false));
    assert!(mark_remediation(
        &connection,
        195,
        remediation,
        "closed",
        "蓝队",
        "看起来好了"
    )
    .expect_err("没有复测不能关闭")
    .contains("exposure_remediation_unclosed"));

    // The retest is a *new* round with its own frozen plan; the old observation is untouched.
    let retest = start_retest(&connection, 195, remediation, &settings).expect("复测启动");
    assert_ne!(retest.run_id, first.run_id, "复测必须另起一轮");
    let status: String = connection
        .query_row(
            "SELECT status FROM exposure_remediations WHERE id=?1",
            [remediation],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "awaiting_retest");
    let old_untouched: String = connection
        .query_row(
            "SELECT content_hash FROM exposure_observations WHERE id=?1",
            [observation],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_untouched, before_hash, "旧观察一个字节都不能被改写");

    // Retest round sees different bytes → the basis may support a close, and the finding goes
    // back to review instead of being called fixed.
    observed(
        &connection,
        retest.run_id,
        195,
        url,
        "clean-page-bytes",
        "泄露的 .env",
    );
    run_plan(
        &connection,
        retest.run_id,
        "owner-e5-b",
        &mut |_item| -> Result<ItemOutcome, String> {
            Ok(ItemOutcome {
                status: Some(WorkItemStatus::Completed),
                source_status: Some(SourceStatus::Completed),
                ..Default::default()
            })
        },
    )
    .expect("复测轮收敛");
    let outcome = retest_outcome(&connection, 195, remediation).expect("复测比对");
    assert_eq!(
        outcome["basis"],
        json!("retest_content_changed"),
        "{outcome}"
    );
    assert_eq!(outcome["canClose"], json!(true));
    assert_ne!(outcome["previousHash"], outcome["currentHash"]);
    let closed = mark_remediation(
        &connection,
        195,
        remediation,
        "closed",
        "蓝队",
        "复测字节已变",
    )
    .expect("关闭");
    assert_eq!(closed["basis"], json!("retest_content_changed"));
    let closed_at: String = connection
        .query_row(
            "SELECT closed_at FROM exposure_remediations WHERE id=?1",
            [remediation],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!closed_at.is_empty(), "关闭必须留下时间");
    let finding_status: String = connection
        .query_row(
            "SELECT status FROM exposure_findings WHERE id=?1",
            [finding],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        finding_status, "reviewing",
        "复测后结论要重做，不能自动算已确认"
    );
    let trail = review_trail(&connection, 195, finding).expect("轨迹");
    assert!(
        trail.iter().any(|row| row["note"]
            .as_str()
            .unwrap_or_default()
            .contains("retest_content_changed")),
        "{trail:?}"
    );
    let history: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_observations WHERE project_id=195",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(history, 2, "两轮各留一条观察");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn e5_identical_unseen_or_failed_retests_never_close_a_remediation() {
    let root = sandbox("refuse");
    let connection = database(&root);
    project(&connection, 196, "范围");
    seed_scope(&connection, 196, "http://u.e5.invalid/", true);
    let settings = current_settings(&connection, 196);
    let first = start_or_reuse(&connection, 196, &settings).expect("第一轮");
    let url = "http://u.e5.invalid/report.pdf";
    let (finding, observation) = observed(
        &connection,
        first.run_id,
        196,
        url,
        "same-bytes",
        "公开报表",
    );
    drain(&connection, first.run_id, "owner-e5-196");
    let remediation = record_remediation(
        &connection,
        196,
        finding,
        Some(observation),
        first.run_id,
        "蓝队",
        "已替换",
        "fixing",
    )
    .expect("整改");

    // A retest round that sees exactly the same bytes proves nothing.
    let retest = start_retest(&connection, 196, remediation, &settings).expect("复测");
    assert_ne!(
        retest.run_id, first.run_id,
        "上一轮已终态时复测必须另起一轮"
    );
    observed(
        &connection,
        retest.run_id,
        196,
        url,
        "same-bytes",
        "公开报表",
    );
    let outcome = retest_outcome(&connection, 196, remediation).expect("比对");
    assert_eq!(
        outcome["basis"],
        json!("retest_content_identical"),
        "{outcome}"
    );
    assert_eq!(outcome["canClose"], json!(false));
    assert!(
        mark_remediation(&connection, 196, remediation, "closed", "蓝队", "一样")
            .expect_err("相同字节不能关闭")
            .contains("exposure_remediation_unclosed")
    );

    // A round that never fetched the URL again is "not seen", not "removed".
    let root2 = sandbox("not-seen");
    let second = database(&root2);
    project(&second, 197, "范围");
    seed_scope(&second, 197, "http://v.e5.invalid/", true);
    let settings2 = current_settings(&second, 197);
    let first2 = start_or_reuse(&second, 197, &settings2).expect("第一轮");
    let (finding2, observation2) = observed(
        &second,
        first2.run_id,
        197,
        "http://v.e5.invalid/secret",
        "bytes-a",
        "秘密文件",
    );
    drain(&second, first2.run_id, "owner-e5-197");
    let remediation2 = record_remediation(
        &second,
        197,
        finding2,
        Some(observation2),
        first2.run_id,
        "蓝队",
        "已删除",
        "fixing",
    )
    .expect("整改");
    let retest2 = start_retest(&second, 197, remediation2, &settings2).expect("复测");
    run_plan(
        &second,
        retest2.run_id,
        "owner-e5-c",
        &mut |_item| -> Result<ItemOutcome, String> {
            Ok(ItemOutcome {
                status: Some(WorkItemStatus::Completed),
                source_status: Some(SourceStatus::Completed),
                ..Default::default()
            })
        },
    )
    .expect("复测轮收敛");
    let outcome2 = retest_outcome(&second, 197, remediation2).expect("比对");
    assert_eq!(outcome2["basis"], json!("retest_not_seen"), "{outcome2}");
    assert!(outcome2["note"]
        .as_str()
        .unwrap_or_default()
        .contains("不能证明已经下线"));
    assert_eq!(outcome2["canClose"], json!(false));

    // A retest round that failed or was interrupted cannot be a basis either.
    run(&second, 19799, 197, "failed");
    second
        .execute(
            "INSERT INTO exposure_remediation_retests(remediation_id,project_id,run_id,finding_id,observation_url) VALUES(?1,197,19799,?2,'http://v.e5.invalid/secret')",
            params![remediation2, finding2],
        )
        .unwrap();
    let outcome3 = retest_outcome(&second, 197, remediation2).expect("比对失败轮");
    assert_eq!(
        outcome3["basis"],
        json!("retest_round_incomplete"),
        "{outcome3}"
    );
    assert_eq!(outcome3["canClose"], json!(false));

    // Rejecting a remediation is a human decision and needs the same actor/reason discipline.
    assert!(
        mark_remediation(&connection, 196, remediation, "rejected", "", "")
            .expect_err("匿名驳回必须拒绝")
            .contains("exposure_remediation_incomplete")
    );
    let rejected = mark_remediation(
        &connection,
        196,
        remediation,
        "rejected",
        "蓝队",
        "误报：公开样例",
    )
    .expect("驳回可以没有复测依据");
    assert_eq!(rejected["basis"], json!("retest_content_identical"));
    // And a terminal remediation cannot be retested again.
    assert!(start_retest(&connection, 196, remediation, &settings)
        .expect_err("已驳回的整改不能再复测")
        .contains("exposure_remediation_terminal"));
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(root2);
}

#[test]
fn e5_a_revoked_scope_cannot_start_a_retest() {
    let root = sandbox("revoked");
    let connection = database(&root);
    project(&connection, 198, "范围");
    seed_scope(&connection, 198, "http://w.e5.invalid/", true);
    let settings = current_settings(&connection, 198);
    let first = start_or_reuse(&connection, 198, &settings).expect("第一轮");
    let (finding, observation) = observed(
        &connection,
        first.run_id,
        198,
        "http://w.e5.invalid/data",
        "bytes",
        "公开数据",
    );
    let remediation = record_remediation(
        &connection,
        198,
        finding,
        Some(observation),
        first.run_id,
        "蓝队",
        "处理中",
        "fixing",
    )
    .expect("整改");
    connection
        .execute(
            "UPDATE asset_ownership_decisions SET exposure_eligible=0, authorization_status='revoked' WHERE project_id=198",
            [],
        )
        .unwrap();
    let error =
        start_retest(&connection, 198, remediation, &settings).expect_err("撤权后不能凭旧授权复测");
    assert!(
        error.contains("exposure_scope") || error.contains("exposure_retest_unauthorized"),
        "拒绝理由必须来自授权门禁：{error}"
    );
    let status: String = connection
        .query_row(
            "SELECT status FROM exposure_remediations WHERE id=?1",
            [remediation],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "fixing", "被拒绝的复测不能改动整改状态");
    let links: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_remediation_retests WHERE remediation_id=?1",
            [remediation],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(links, 0, "也不能留下一个不存在的复测轮");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn e5_the_taxonomy_keeps_no_evidence_apart_from_no_problem() {
    let root = sandbox("taxonomy");
    let connection = database(&root);
    project(&connection, 199, "范围");
    run(&connection, 19900, 199, "completed");
    let (confirmed, observation) = observed(
        &connection,
        19900,
        199,
        "https://x.e5.invalid/a",
        "bytes-a",
        "已确认的一条",
    );
    review_scoped(
        &connection,
        199,
        confirmed,
        top_revision(&connection, confirmed).unwrap(),
        "confirmed",
        "王凡",
        "两处证据一致",
        &[observation],
    )
    .expect("确认");
    let (dismissed, _) = observed(
        &connection,
        19900,
        199,
        "https://x.e5.invalid/b",
        "bytes-b",
        "误报的一条",
    );
    let top = top_revision(&connection, dismissed).unwrap();
    review_scoped(
        &connection,
        199,
        dismissed,
        top,
        "dismissed",
        "王凡",
        "公开样例",
        &[],
    )
    .expect("排除");
    // A candidate with no observation behind it at all.
    connection
        .execute(
            "INSERT INTO exposure_findings(project_id,category,title,source_type,source_url,evidence_excerpt,severity,confidence,status,fingerprint)
             VALUES(199,'credential','没有证据的猜测','manual','','','high',40,'new','fp-no-evidence')",
            [],
        )
        .unwrap();
    // A failed source and an uncovered work item.
    connection
        .execute(
            "INSERT INTO exposure_source_results(run_id,source_key,status,item_count,error) VALUES(19900,'certificate_transparency','failed',0,'索引不可达')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO exposure_work_items(run_id,project_id,attempt_number,work_key,kind,source_key,status,reason)
             VALUES(19900,199,1,'passive_web|uncovered','passive_web','passive_web','blocked_quota','本轮工作项额度 1，剩余 1 项未采集')",
            [],
        )
        .unwrap();
    let rows = taxonomy(&connection, 199).expect("分类");
    let by_code: std::collections::BTreeMap<String, i64> = rows
        .iter()
        .map(|row| (row.code.clone(), row.count))
        .collect();
    assert_eq!(by_code["confirmed_exposure"], 1, "{by_code:?}");
    assert_eq!(by_code["dismissed"], 1);
    assert_eq!(
        by_code["review_pending"], 1,
        "待复核只数 new/reviewing 两条状态里的条目"
    );
    assert_eq!(by_code["source_unreachable"], 1);
    assert_eq!(by_code["not_covered"], 1);
    assert_eq!(
        by_code["unknown"], 1,
        "没有观察支撑的条目必须单列，不能被算成结论"
    );
    assert_eq!(by_code["rule_hit"], 2, "两条有版本行的规则命中");
    // Every row says what it means; a bare number is not an answer.
    for row in &rows {
        assert!(!row.label.is_empty() && !row.meaning.is_empty(), "{row:?}");
    }
    let _ = std::fs::remove_dir_all(root);
}

/// The page's CSV and the reviewed queue have to agree; this checks the reviewed status is what
/// the export carries, so "已确认" cannot show on one side only.
#[test]
fn e5_the_csv_export_reflects_the_reviewed_status_of_each_row() {
    let root = sandbox("csv");
    let connection = database(&root);
    project(&connection, 201, "范围");
    run(&connection, 20100, 201, "completed");
    let (a, observation) = observed(
        &connection,
        20100,
        201,
        "https://y.e5.invalid/a",
        "bytes-a",
        "甲",
    );
    let (_, _) = observed(
        &connection,
        20100,
        201,
        "https://y.e5.invalid/b",
        "bytes-b",
        "乙",
    );
    review_scoped(
        &connection,
        201,
        a,
        top_revision(&connection, a).unwrap(),
        "confirmed",
        "王凡",
        "已核对原始响应",
        &[observation],
    )
    .expect("确认");
    // A credential-shaped excerpt on one row: the export has to mask it like the page does.
    connection
        .execute(
            "UPDATE exposure_findings SET evidence_excerpt=?1 WHERE id=?2",
            params!["api_key = SUPERSECRETVALUE1", a],
        )
        .unwrap();
    let rows = super::export::finding_rows(&connection, 201).expect("导出行");
    let joined = rows.iter().flatten().cloned().collect::<Vec<_>>().join("|");
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(
        joined.contains("confirmed"),
        "页面确认后导出要同一状态：{joined}"
    );
    assert!(
        !joined.contains("SUPERSECRETVALUE1"),
        "导出必须与页面一样脱敏：{joined}"
    );
    let taxonomy_rows = taxonomy(&connection, 201).expect("分类");
    let confirmed = taxonomy_rows
        .iter()
        .find(|row| row.code == "confirmed_exposure")
        .unwrap();
    assert_eq!(confirmed.count, 1, "页面与导出的口径必须一致");
    let _ = std::fs::remove_dir_all(root);
}

/// A probe used by the tests above to keep the module's own close helper honest.
#[allow(dead_code)]
fn close_remediation_probe() {}
