// O3 验收测试：增量、崩溃恢复、分块写入、组级撤销、撤销授权、拆分确认/授权合同、并发与规模。
#[cfg(test)]
mod ownership_o3_tests {
    use super::ownership_review_o2_tests::{
        apply_input, count, decision, fixture, group_of, percentile, request, run_analysis,
    };
    use super::*;

    const DEAD_TOKEN: &str = "deadbeef0000:previous-process";

    /// 把夹具数据的水位放到很早以前（模拟历史导入），使增量可以沿用未变化行。
    fn age_all(connection: &rusqlite::Connection) {
        connection
            .execute_batch(
                "UPDATE assets SET last_seen='2000-01-01 00:00:00';
                 UPDATE project_assets SET last_seen='2000-01-01 00:00:00';",
            )
            .unwrap();
    }

    /// 模拟导入器写入新证据：改字段并推进 last_seen（jobs.rs 导入 upsert 的行为）。
    fn touch(connection: &rusqlite::Connection, asset_id: i64, company: &str) {
        connection
            .execute(
                "UPDATE assets SET company=?2,last_seen=datetime('now','localtime') WHERE id=?1",
                params![asset_id, company],
            )
            .unwrap();
    }

    fn add_zj_assets(connection: &rusqlite::Connection, ids: std::ops::RangeInclusive<i64>) {
        for id in ids {
            connection
                .execute(
                    "INSERT INTO assets(id,asset_key,company,host,domain,link,title) VALUES(?1,?2,'中国移动通信集团浙江有限公司','zj.10086.cn','zj.10086.cn','https://zj.10086.cn/p'||?1,'t')",
                    params![id, format!("o3-{id}")],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO project_assets(project_id,asset_id) VALUES(91,?1)",
                    [id],
                )
                .unwrap();
        }
    }

    fn add_asset(connection: &rusqlite::Connection, id: i64, company: &str, host: &str) {
        connection
            .execute(
                "INSERT INTO assets(id,asset_key,company,host,domain,link,title) VALUES(?1,?2,?3,?4,?4,'https://'||?4||'/p'||?1,'t')",
                params![id, format!("o3-{id}"), company, host],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO project_assets(project_id,asset_id) VALUES(91,?1)",
                [id],
            )
            .unwrap();
    }

    fn run_mode(
        connection: &mut rusqlite::Connection,
        mode: &str,
        chunk: i64,
    ) -> OwnershipAnalysisRunView {
        let (run, needs_worker) = start_ownership_analysis_with_mode(connection, 91, mode).unwrap();
        assert!(needs_worker);
        let token = ownership_claim_run(connection, run.id).unwrap().expect("claim");
        assert_eq!(
            ownership_drive_run(connection, run.id, &token, chunk, None).unwrap(),
            OwnershipStep::Completed
        );
        let done = ownership_read_run(connection, run.id).unwrap().unwrap();
        assert_eq!(done.status, "completed", "{}", done.error);
        done
    }

    /// 快照逐资产内容：(资产, 组键, 证据指纹, 结论状态)。
    fn snapshot_rows(connection: &rusqlite::Connection, run_id: i64) -> Vec<(i64, String, String, String)> {
        let mut statement = connection
            .prepare("SELECT m.asset_id,g.group_key,m.evidence_hash,m.decision_state FROM ownership_review_group_members m JOIN ownership_review_groups g ON g.id=m.group_id WHERE m.run_id=?1 ORDER BY m.asset_id")
            .unwrap();
        let rows = statement
            .query_map([run_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        rows
    }

    fn group_counts(connection: &rusqlite::Connection, run_id: i64) -> Vec<(String, i64)> {
        let mut statement = connection
            .prepare("SELECT group_key,member_count FROM ownership_review_groups WHERE run_id=?1 ORDER BY group_key")
            .unwrap();
        let rows = statement
            .query_map([run_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        rows
    }

    fn scope_total(connection: &rusqlite::Connection) -> i64 {
        count(connection, &format!("SELECT COUNT(*) FROM project_assets pa JOIN assets a ON a.id=pa.asset_id LEFT JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=pa.asset_id WHERE pa.project_id=91 AND {OWNERSHIP_SCOPE_SQL}"))
    }

    fn expectation(connection: &rusqlite::Connection, asset_id: i64) -> AssetOwnershipDecisionExpectation {
        ownership_expectation_of(connection, 91, asset_id).unwrap()
    }

    fn selection(asset_id: i64) -> AssetSelection {
        AssetSelection {
            project_id: 91,
            asset_id,
        }
    }

    fn manual(connection: &mut rusqlite::Connection, asset_id: i64, status: &str) {
        let expected = vec![expectation(connection, asset_id)];
        apply_asset_ownership_decisions(
            connection,
            AssetOwnershipDecisionInput {
                selections: vec![selection(asset_id)],
                status: status.into(),
                note: "人工".into(),
                learn_domain_rule: false,
            },
            91,
            expected,
        )
        .unwrap();
    }

    fn exposure(connection: &rusqlite::Connection) -> crate::commands::exposure::scope::ExposureScope {
        crate::commands::exposure::scope::ExposureScope::load(connection, 91, &serde_json::json!({}))
            .unwrap_or_else(|denial| panic!("{}", denial.message))
    }

    /// None = 资产在下游暴露面授权范围内；Some(reason) = 被排除。
    fn exposure_exclusion(connection: &rusqlite::Connection, asset_id: i64) -> Option<&'static str> {
        exposure(connection)
            .excluded
            .iter()
            .find(|item| item.asset_id == asset_id)
            .map(|item| item.reason)
    }

    fn granted(connection: &rusqlite::Connection, url: &str) -> bool {
        exposure(connection)
            .grant_for(&reqwest::Url::parse(url).unwrap())
            .is_some()
    }

    fn group_state(connection: &rusqlite::Connection, run_id: i64, group_id: i64) -> (i64, i64, i64) {
        connection
            .query_row(
                "SELECT group_version,applied_operations,authorization_revoked FROM ownership_review_groups WHERE id=?1 AND run_id=?2",
                params![group_id, run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap()
    }

    fn eligible_rows(connection: &rusqlite::Connection) -> i64 {
        count(connection, "SELECT COUNT(*) FROM asset_ownership_decisions WHERE project_id=91 AND (exposure_eligible=1 OR authorization_status='allowed')")
    }

    fn op_events(connection: &rusqlite::Connection, operation_id: &str) -> i64 {
        connection
            .query_row(
                "SELECT COUNT(*) FROM asset_events WHERE summary LIKE '%'||?1||'%'",
                [operation_id],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn owned(begin: OwnershipBegin) -> String {
        match begin {
            OwnershipBegin::Owned(token) => token,
            OwnershipBegin::Existing => panic!("expected to own the operation"),
        }
    }

    fn cleanup(root: PathBuf, connection: rusqlite::Connection) {
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    // ---------------------------------------------------------------- 1. 增量
    #[test]
    fn incremental_snapshot_equals_full_and_never_touches_the_base() {
        let (root, mut connection) = fixture();
        age_all(&connection);
        let base = run_mode(&mut connection, "auto", 4);
        assert_eq!((base.run_kind.as_str(), base.mode_reason.as_str()), ("full", "no_snapshot"));
        assert!(!base.watermark_at.is_empty());
        // 基线上已提交的一次组操作及其结果必须保持原样。
        let group = group_of(&connection, base.id, 1).id;
        let preview = preview_ownership_group_decision_for_connection(&connection, request(base.id, group, "confirm_ownership")).unwrap();
        apply_ownership_group_decision_for_connection(&mut connection, apply_input(&preview, "op-o3-base-0001")).unwrap();
        let base_rows = snapshot_rows(&connection, base.id);
        let base_groups = group_counts(&connection, base.id);
        let base_result: String = connection.query_row("SELECT result_json||status FROM ownership_operations WHERE operation_id='op-o3-base-0001'", [], |row| row.get(0)).unwrap();
        // 水位之后的数据变化：证据更新、删除、人工结论、新资产。
        touch(&connection, 5, "中国联合网络通信集团有限公司");
        connection.execute("UPDATE project_assets SET is_deleted=1 WHERE project_id=91 AND asset_id=3", []).unwrap();
        manual(&mut connection, 12, "excluded");
        add_asset(&connection, 40, ZJ_NAME, "new.zj.10086.cn");
        let incremental = run_mode(&mut connection, "auto", 4);
        assert_eq!(incremental.run_kind, "incremental", "{}", incremental.mode_reason);
        assert_eq!(incremental.base_run_id, Some(base.id));
        assert_eq!(incremental.carried_assets + incremental.reclassified_assets, incremental.processed_assets);
        assert_eq!(incremental.processed_assets, scope_total(&connection));
        // 重新分类的 = 基线没有或结论状态不同的成员 + 证据变化的 5 号；其余全部沿用。
        let changed_state = count(&connection, &format!("SELECT COUNT(*) FROM ownership_review_group_members m WHERE m.run_id={} AND NOT EXISTS(SELECT 1 FROM ownership_review_group_members b WHERE b.run_id={} AND b.asset_id=m.asset_id AND b.decision_state=m.decision_state)", incremental.id, base.id));
        assert!(changed_state >= 4, "group decision rows, manual 12, new 40: {changed_state}");
        assert_eq!(incremental.reclassified_assets, changed_state + 1);
        assert!(incremental.carried_assets > 0);
        assert!(snapshot_rows(&connection, incremental.id).iter().all(|row| row.0 != 3), "deleted member is gone");
        // 基线快照、它的组和已提交操作不变；基于基线的新预览被拒绝。
        assert_eq!(snapshot_rows(&connection, base.id), base_rows);
        assert_eq!(group_counts(&connection, base.id), base_groups);
        let after: String = connection.query_row("SELECT result_json||status FROM ownership_operations WHERE operation_id='op-o3-base-0001'", [], |row| row.get(0)).unwrap();
        assert_eq!(after, base_result);
        let stale = preview_ownership_group_decision_for_connection(&connection, request(base.id, group, "confirm_ownership")).unwrap_err();
        assert!(stale.starts_with(OWNERSHIP_SNAPSHOT_STALE), "{stale}");
        // 与同一时刻的全量分析逐资产、逐组一致。
        let full = run_mode(&mut connection, "full", 4);
        assert_eq!((full.run_kind.as_str(), full.mode_reason.as_str()), ("full", "requested_full"));
        assert_eq!(full.reclassified_assets, full.processed_assets);
        assert_eq!(snapshot_rows(&connection, incremental.id), snapshot_rows(&connection, full.id));
        assert_eq!(group_counts(&connection, incremental.id), group_counts(&connection, full.id));
        cleanup(root, connection);
    }

    const ZJ_NAME: &str = "中国移动通信集团浙江有限公司";

    #[test]
    fn changes_made_while_an_analysis_runs_are_reclassified_by_the_next_increment() {
        let (root, mut connection) = fixture();
        age_all(&connection);
        let (run, _) = start_ownership_analysis_with_mode(&mut connection, 91, "auto").unwrap();
        let token = ownership_claim_run(&mut connection, run.id).unwrap().unwrap();
        assert_eq!(ownership_drive_run(&mut connection, run.id, &token, 3, Some(2)).unwrap(), OwnershipStep::Continued);
        // 已处理过的 1、2 号在分析进行中被更新（证据 / 人工结论），尚未处理的 16 号也被更新。
        touch(&connection, 1, "中国移动通信集团江苏有限公司");
        manual(&mut connection, 2, "related");
        touch(&connection, 16, "中国移动通信集团浙江有限公司");
        assert_eq!(ownership_drive_run(&mut connection, run.id, &token, 3, None).unwrap(), OwnershipStep::Completed);
        let base = ownership_read_run(&connection, run.id).unwrap().unwrap();
        assert_eq!((base.processed_assets, base.member_count), (base.total_assets, base.total_assets));
        let incremental = run_mode(&mut connection, "auto", 3);
        assert_eq!(incremental.run_kind, "incremental");
        let carried: HashSet<i64> = {
            let base_rows: HashMap<i64, (String, String, String)> = snapshot_rows(&connection, base.id).into_iter().map(|row| (row.0, (row.1, row.2, row.3))).collect();
            snapshot_rows(&connection, incremental.id).into_iter().filter(|row| base_rows.get(&row.0) == Some(&(row.1.clone(), row.2.clone(), row.3.clone()))).map(|row| row.0).collect()
        };
        // 1、2 号在基线处理之后才变化：基线内容已过时，增量必须重新分类（内容不同于基线）。
        for asset in [1, 2] {
            assert!(!carried.contains(&asset), "asset {asset} changed after it was processed");
        }
        // 重新分类恰好是 {1（证据）, 2（人工结论）, 16（水位后证据）}，其余全部沿用。
        assert_eq!(incremental.reclassified_assets, 3);
        assert_eq!(incremental.carried_assets, incremental.processed_assets - 3);
        let full = run_mode(&mut connection, "full", 3);
        assert_eq!(snapshot_rows(&connection, incremental.id), snapshot_rows(&connection, full.id));
        cleanup(root, connection);
    }

    #[test]
    fn rule_directory_and_profile_changes_force_a_full_rerun_with_the_reason() {
        let (root, mut connection) = fixture();
        age_all(&connection);
        run_mode(&mut connection, "auto", 50);
        connection.execute("INSERT INTO asset_ownership_rules(project_id,rule_type,pattern,action,source) VALUES(91,'host','random.example.org','approve','manual_decision')", []).unwrap();
        let after_rule = run_mode(&mut connection, "auto", 50);
        assert_eq!(after_rule.run_kind, "full");
        assert!(after_rule.mode_reason.starts_with("versions_changed:") && after_rule.mode_reason.contains("rules"), "{}", after_rule.mode_reason);
        let unchanged = run_mode(&mut connection, "auto", 50);
        assert_eq!((unchanged.run_kind.as_str(), unchanged.base_run_id), ("incremental", Some(after_rule.id)));
        assert_eq!(unchanged.reclassified_assets, 0, "nothing changed since the base");
        connection.execute("UPDATE org_directory_state SET version=version+1", []).unwrap();
        let after_directory = run_mode(&mut connection, "auto", 50);
        assert!(after_directory.run_kind == "full" && after_directory.mode_reason.contains("directory"), "{}", after_directory.mode_reason);
        // 排队中的增量在开始前遇到规则变化：改为全量并重新绑定版本。
        let (queued, _) = start_ownership_analysis_with_mode(&mut connection, 91, "auto").unwrap();
        assert_eq!(queued.run_kind, "incremental");
        connection.execute("UPDATE asset_ownership_rules SET enabled=0 WHERE project_id=91", []).unwrap();
        let token = ownership_claim_run(&mut connection, queued.id).unwrap().unwrap();
        let rebound = ownership_read_run(&connection, queued.id).unwrap().unwrap();
        assert_eq!((rebound.run_kind.as_str(), rebound.base_run_id), ("full", None));
        assert!(rebound.mode_reason.starts_with("versions_changed_before_start:") && rebound.mode_reason.contains("rules"), "{}", rebound.mode_reason);
        assert_eq!(rebound.versions, ownership_current_versions(&connection, 91).unwrap());
        assert_eq!(ownership_drive_run(&mut connection, queued.id, &token, 50, None).unwrap(), OwnershipStep::Completed);
        // 已处理一部分后版本变化：续跑不能拼接两套规则的结果。
        let (run, _) = start_ownership_analysis_with_mode(&mut connection, 91, "full").unwrap();
        let token = ownership_claim_run(&mut connection, run.id).unwrap().unwrap();
        ownership_drive_run(&mut connection, run.id, &token, 3, Some(1)).unwrap();
        connection.execute("UPDATE asset_ownership_rules SET enabled=1 WHERE project_id=91", []).unwrap();
        connection.execute("UPDATE ownership_analysis_runs SET worker_token=?2 WHERE id=?1", params![run.id, DEAD_TOKEN]).unwrap();
        assert_eq!(ownership_claim_run(&mut connection, run.id).unwrap(), None);
        let failed = ownership_read_run(&connection, run.id).unwrap().unwrap();
        assert_eq!(failed.status, "failed");
        assert_eq!(count(&connection, &format!("SELECT COUNT(*) FROM ownership_review_group_members WHERE run_id={}", run.id)), 0);
        cleanup(root, connection);
    }

    // ---------------------------------------------------------------- 2. 崩溃恢复（分析）
    #[test]
    fn startup_recovery_takes_over_an_interrupted_analysis_with_real_counts() {
        let (root, mut connection) = fixture();
        let (run, _) = start_ownership_analysis_with_mode(&mut connection, 91, "full").unwrap();
        let token = ownership_claim_run(&mut connection, run.id).unwrap().unwrap();
        assert_eq!(ownership_drive_run(&mut connection, run.id, &token, 3, Some(2)).unwrap(), OwnershipStep::Continued);
        let partial = ownership_read_run(&connection, run.id).unwrap().unwrap();
        assert_eq!(partial.processed_assets, 6);
        // 应用重启：令牌属于已退出的进程，心跳仍“新鲜”。
        connection.execute("UPDATE ownership_analysis_runs SET worker_token=?2 WHERE id=?1", params![run.id, DEAD_TOKEN]).unwrap();
        let (same, needs_worker) = start_ownership_analysis_with_mode(&mut connection, 91, "auto").unwrap();
        assert_eq!((same.id, needs_worker), (run.id, true), "no duplicate run, takeover requested");
        let report = ownership_recover_for_connection(&mut connection).unwrap();
        assert_eq!(report.runs_resumed, vec![run.id]);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        let done = ownership_read_run(&connection, run.id).unwrap().unwrap();
        let scope = scope_total(&connection);
        assert_eq!(done.status, "completed");
        assert_eq!((done.processed_assets, done.total_assets, done.member_count, done.resume_count), (scope, scope, scope, 1));
        assert_eq!(count(&connection, &format!("SELECT COUNT(*) FROM ownership_review_group_members WHERE run_id={}", run.id)), scope);
        assert_eq!(count(&connection, &format!("SELECT COALESCE(SUM(member_count),0) FROM ownership_review_groups WHERE run_id={}", run.id)), scope);
        assert_eq!(ownership_drive_run(&mut connection, run.id, DEAD_TOKEN, 3, None).unwrap(), OwnershipStep::Lost);
        assert!(ownership_recover_for_connection(&mut connection).unwrap().runs_resumed.is_empty());
        cleanup(root, connection);
    }

    // ---------------------------------------------------------------- 3. 大组分块写入
    #[test]
    fn chunked_group_apply_survives_a_crash_without_double_writes_or_partial_allow() {
        let (root, mut connection) = fixture();
        add_zj_assets(&connection, 41..=46);
        // 一半成员已有（未授权的）结论行：分块写入要同时覆盖 UPDATE 与 INSERT 两条路径。
        for asset in 41..=43 {
            connection
                .execute("INSERT INTO asset_ownership_decisions(project_id,asset_id,status,confidence,authorization_status,exposure_eligible,source,reason,evidence_json,negative_evidence_json) VALUES(91,?1,'unreviewed',0,'unknown',0,'auto','','[]','[]')", [asset])
                .unwrap();
        }
        let run = run_analysis(&mut connection, 91, 50);
        let group = group_of(&connection, run, 1).id;
        let preview = preview_ownership_group_decision_for_connection(&connection, request(run, group, "confirm_ownership")).unwrap();
        assert!(preview.applicable >= 8, "{}", preview.applicable);
        let op = "op-o3-chunk-0001";
        let eligible_before = eligible_rows(&connection);
        let version_before = group_state(&connection, run, group);
        let token = owned(ownership_begin_group_decision(&mut connection, &apply_input(&preview, op), op).unwrap());
        assert_eq!(ownership_drive_operation(&mut connection, op, &token, 2, None, Some(1)).unwrap(), OwnershipStep::Continued);
        let partial = read_ownership_operation_for_connection(&connection, 91, op).unwrap();
        assert_eq!((partial.status.as_str(), partial.done_items, partial.applied), ("applying", 2, 2));
        assert_eq!(partial.total_items, preview.member_count);
        // 未完成批次不向下游发布任何“允许”，组也不标记为已处理。
        assert_eq!(eligible_rows(&connection), eligible_before);
        assert_eq!(group_state(&connection, run, group), version_before);
        // 存活工作线程持有时重放：只返回真实进度，不重复写。
        let (replay, takeover) = apply_ownership_group_decision_budgeted(&mut connection, apply_input(&preview, op), Some(Duration::ZERO), 2).unwrap();
        assert!(replay.replayed && takeover.is_none());
        assert_eq!((replay.status.as_str(), replay.applied), ("applying", 2));
        // 进程崩溃：下次启动接管。
        connection.execute("UPDATE ownership_operations SET worker_token=?2 WHERE operation_id=?1", params![op, DEAD_TOKEN]).unwrap();
        let report = ownership_recover_for_connection(&mut connection).unwrap();
        assert_eq!(report.operations_resumed, vec![op.to_string()]);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        let done = read_ownership_operation_for_connection(&connection, 91, op).unwrap();
        assert_eq!((done.status.as_str(), done.applied, done.failed, done.resume_count), ("completed", preview.applicable, 0, 1));
        assert_eq!((done.done_items, done.applied + done.skipped + done.conflicted), (done.total_items, done.total_items));
        assert_eq!(count(&connection, &format!("SELECT COUNT(DISTINCT asset_id) FROM ownership_operation_items WHERE operation_id='{op}'")), done.total_items);
        assert_eq!(count(&connection, &format!("SELECT COUNT(*) FROM ownership_operation_items WHERE operation_id='{op}'")), done.total_items);
        assert_eq!(op_events(&connection, op), preview.applicable, "one audit event per written row");
        assert_eq!(count(&connection, &format!("SELECT COUNT(*) FROM ownership_operation_staging WHERE operation_id='{op}'")), 0);
        assert_eq!(eligible_rows(&connection), eligible_before, "group confirmation never grants");
        let applied_paths = |exists: i64| count(&connection, &format!("SELECT COUNT(*) FROM ownership_operation_items WHERE operation_id='{op}' AND outcome='applied' AND json_extract(old_json,'$.exists')={exists}"));
        assert!(applied_paths(1) >= 3 && applied_paths(0) >= 1, "both UPDATE ({}) and INSERT ({}) paths were written", applied_paths(1), applied_paths(0));
        assert_eq!(count(&connection, "SELECT COUNT(*) FROM asset_ownership_decisions WHERE project_id=91 AND asset_id BETWEEN 41 AND 46 AND (authorization_status<>'unknown' OR exposure_eligible<>0 OR status<>'confirmed')"), 0);
        assert_eq!(group_state(&connection, run, group).1, version_before.1 + 1);
        assert_eq!(ownership_operation_step(&mut connection, op, DEAD_TOKEN, 2).unwrap(), OwnershipStep::Lost);
        let replay = apply_ownership_group_decision_for_connection(&mut connection, apply_input(&preview, op)).unwrap();
        assert!(replay.replayed);
        assert_eq!((replay.status.as_str(), replay.applied), ("completed", preview.applicable));
        assert_eq!(op_events(&connection, op), preview.applicable);
        cleanup(root, connection);
    }

    #[test]
    fn recovery_never_restores_a_revocation_made_during_an_unfinished_batch() {
        let (root, mut connection) = fixture();
        add_zj_assets(&connection, 41..=46);
        let run = run_analysis(&mut connection, 91, 50);
        let group = group_of(&connection, run, 1).id;
        let preview = preview_ownership_group_decision_for_connection(&connection, request(run, group, "confirm_ownership")).unwrap();
        let op = "op-o3-revoke-mid-0001";
        let token = owned(ownership_begin_group_decision(&mut connection, &apply_input(&preview, op), op).unwrap());
        ownership_drive_operation(&mut connection, op, &token, 2, None, Some(1)).unwrap();
        let first_chunk: Vec<i64> = {
            let mut statement = connection.prepare("SELECT asset_id FROM ownership_operation_items WHERE operation_id=?1 AND outcome='applied'").unwrap();
            let rows = statement.query_map([op], |row| row.get(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
            rows
        };
        assert_eq!(first_chunk.len(), 2);
        let revoked = revoke_ownership_group_authorization_for_connection(&mut connection, OwnershipRevokeRequest { operation_id: "op-o3-rvk-mid-0001".into(), project_id: 91, run_id: run, group_id: group, note: String::new() }, false).unwrap();
        assert_eq!(revoked.revoked, 2, "rows written so far are revoked; rows without a decision are skipped");
        connection.execute("UPDATE ownership_operations SET worker_token=?2 WHERE operation_id=?1", params![op, DEAD_TOKEN]).unwrap();
        ownership_recover_for_connection(&mut connection).unwrap();
        for asset in &first_chunk {
            let (_, _, authorization, eligible) = decision(&connection, *asset).unwrap();
            assert_eq!((authorization.as_str(), eligible), ("revoked", 0), "asset {asset}");
        }
        assert_eq!(count(&connection, "SELECT COUNT(*) FROM asset_ownership_decisions WHERE project_id=91 AND authorization_status='allowed' AND asset_id NOT IN (14,15)"), 0);
        let done = read_ownership_operation_for_connection(&connection, 91, op).unwrap();
        assert_eq!(done.status, "completed");
        cleanup(root, connection);
    }

    // ---------------------------------------------------------------- 6. 并发变更
    #[test]
    fn concurrent_applies_and_manual_changes_never_double_write() {
        let (root, mut connection) = fixture();
        add_zj_assets(&connection, 41..=46);
        let run = run_analysis(&mut connection, 91, 50);
        let group = group_of(&connection, run, 1).id;
        let preview = preview_ownership_group_decision_for_connection(&connection, request(run, group, "confirm_ownership")).unwrap();
        let op = "op-o3-a-0001";
        let token = owned(ownership_begin_group_decision(&mut connection, &apply_input(&preview, op), op).unwrap());
        let busy = apply_ownership_group_decision_for_connection(&mut connection, apply_input(&preview, "op-o3-b-0001")).unwrap_err();
        assert!(busy.starts_with(OWNERSHIP_OPERATION_BUSY), "{busy}");
        assert_eq!(count(&connection, "SELECT COUNT(*) FROM ownership_operations WHERE operation_id='op-o3-b-0001'"), 0);
        ownership_drive_operation(&mut connection, op, &token, 2, None, Some(1)).unwrap();
        let later: Vec<i64> = {
            let mut statement = connection.prepare("SELECT asset_id FROM ownership_operation_staging WHERE operation_id=?1 AND seq>2 AND planned='apply' ORDER BY seq LIMIT 2").unwrap();
            let rows = statement.query_map([op], |row| row.get(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
            rows
        };
        assert_eq!(later.len(), 2);
        manual(&mut connection, later[0], "excluded");
        connection.execute("UPDATE project_assets SET is_deleted=1 WHERE project_id=91 AND asset_id=?1", [later[1]]).unwrap();
        assert_eq!(ownership_drive_operation(&mut connection, op, &token, 2, None, None).unwrap(), OwnershipStep::Completed);
        let done = read_ownership_operation_for_connection(&connection, 91, op).unwrap();
        assert_eq!(done.reasons.get("decision_changed"), Some(&1));
        assert_eq!(done.reasons.get("deleted_or_unlinked"), Some(&1));
        assert_eq!(done.applied, preview.applicable - 2);
        let (status, source, authorization, _) = decision(&connection, later[0]).unwrap();
        assert_eq!((status.as_str(), source.as_str(), authorization.as_str()), ("excluded", "manual", "denied"), "the person's later write wins");
        assert_eq!(decision(&connection, later[1]), None, "deleted member untouched");
        let stale = apply_ownership_group_decision_for_connection(&mut connection, apply_input(&preview, "op-o3-b-0001")).unwrap_err();
        assert!(stale.starts_with(OWNERSHIP_PREVIEW_STALE), "{stale}");
        cleanup(root, connection);
    }

    #[test]
    fn parallel_workers_apply_one_group_exactly_once() {
        let (root, mut connection) = fixture();
        add_zj_assets(&connection, 41..=46);
        let run = run_analysis(&mut connection, 91, 50);
        let path = PathBuf::from(connection.path().unwrap());
        // 不同 operationId 同时提交同一组：只有一个生效。
        let group = group_of(&connection, run, 1).id;
        let preview = preview_ownership_group_decision_for_connection(&connection, request(run, group, "confirm_ownership")).unwrap();
        let handles: Vec<_> = (0..4)
            .map(|index| {
                let path = path.clone();
                let input = apply_input(&preview, &format!("op-o3-par-000{index}"));
                thread::spawn(move || {
                    let mut connection = db::open(&path).unwrap();
                    apply_ownership_group_decision_budgeted(&mut connection, input, None, 1).map(|(result, _)| result)
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|handle| handle.join().unwrap()).collect();
        let winners: Vec<_> = results.iter().filter_map(|result| result.as_ref().ok()).collect();
        assert_eq!(winners.len(), 1, "{results:?}");
        for error in results.iter().filter_map(|result| result.as_ref().err()) {
            assert!(error.starts_with(OWNERSHIP_OPERATION_BUSY) || error.starts_with(OWNERSHIP_PREVIEW_STALE), "{error}");
        }
        assert_eq!(winners[0].applied, preview.applicable);
        assert_eq!(count(&connection, &format!("SELECT COUNT(*) FROM ownership_operation_items i JOIN ownership_operations o ON o.operation_id=i.operation_id WHERE o.group_id={group} AND o.kind='group_decision'")), preview.member_count);
        // 同一 operationId 被并发重放：恰好执行一次。
        let other = group_of(&connection, run, 10).id;
        let preview = preview_ownership_group_decision_for_connection(&connection, request(run, other, "excluded")).unwrap();
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let path = path.clone();
                let input = apply_input(&preview, "op-o3-same-0001");
                thread::spawn(move || {
                    let mut connection = db::open(&path).unwrap();
                    apply_ownership_group_decision_budgeted(&mut connection, input, None, 1).map(|(result, _)| result)
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|handle| handle.join().unwrap().unwrap()).collect();
        assert_eq!(results.iter().filter(|result| !result.replayed).count(), 1);
        assert_eq!(op_events(&connection, "op-o3-same-0001"), preview.applicable);
        assert_eq!(count(&connection, "SELECT COUNT(*) FROM ownership_operation_items WHERE operation_id='op-o3-same-0001'"), preview.member_count);
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    // ---------------------------------------------------------------- 4. 组级撤销 / 撤销授权
    #[test]
    fn group_revert_restores_ownership_facts_but_never_authorization() {
        let (root, mut connection) = fixture();
        let run = run_analysis(&mut connection, 91, 50);
        // 15 号：历史遗留 attributed/auto/allowed/1。整组排除后再撤销，不得恢复允许/准入。
        let group = group_of(&connection, run, 15).id;
        let preview = preview_ownership_group_decision_for_connection(&connection, request(run, group, "excluded")).unwrap();
        apply_ownership_group_decision_for_connection(&mut connection, apply_input(&preview, "op-o3-excl-0001")).unwrap();
        assert_eq!(decision(&connection, 15), Some(("excluded".into(), "manual".into(), "denied".into(), 0)));
        let request_revert = OwnershipRevertRequest { project_id: 91, target_operation_id: "op-o3-excl-0001".into(), note: String::new() };
        let revert_preview = preview_ownership_group_revert_for_connection(&connection, request_revert.clone()).unwrap();
        assert_eq!((revert_preview.revertible, revert_preview.authorization_not_restored), (1, 1));
        let apply = |preview: &OwnershipRevertPreview, operation_id: &str| OwnershipRevertApply {
            operation_id: operation_id.into(),
            project_id: 91,
            target_operation_id: preview.target_operation_id.clone(),
            note: String::new(),
            expected_revert_hash: preview.expected_revert_hash.clone(),
        };
        let (result, _) = apply_ownership_group_revert_budgeted(&mut connection, apply(&revert_preview, "op-o3-rv-0001"), None, 500).unwrap();
        assert_eq!((result.status.as_str(), result.applied), ("completed", 1));
        assert_eq!(result.reasons.get("authorization_not_restored"), Some(&1));
        assert_eq!(decision(&connection, 15), Some(("attributed".into(), "auto".into(), "unknown".into(), 0)));
        assert_eq!(exposure_exclusion(&connection, 15), Some("not_exposure_eligible"));
        let twice = preview_ownership_group_revert_for_connection(&connection, request_revert).unwrap_err();
        assert!(twice.starts_with(OWNERSHIP_REVERT_DENIED), "{twice}");
        let of_revert = preview_ownership_group_revert_for_connection(&connection, OwnershipRevertRequest { project_id: 91, target_operation_id: "op-o3-rv-0001".into(), note: String::new() }).unwrap_err();
        assert!(of_revert.starts_with(OWNERSHIP_REVERT_DENIED), "{of_revert}");
        let foreign = preview_ownership_group_revert_for_connection(&connection, OwnershipRevertRequest { project_id: 92, target_operation_id: "op-o3-excl-0001".into(), note: String::new() }).unwrap_err();
        assert!(foreign.starts_with(OWNERSHIP_REVIEW_INVALID), "{foreign}");
        // 撤销整组确认：新建的结论行被删除；之后被人改过的成员保留现状。
        let group = group_of(&connection, run, 1).id;
        let preview = preview_ownership_group_decision_for_connection(&connection, request(run, group, "confirm_ownership")).unwrap();
        let group_before = group_state(&connection, run, group);
        let confirmed = apply_ownership_group_decision_for_connection(&mut connection, apply_input(&preview, "op-o3-conf-0001")).unwrap();
        manual(&mut connection, 2, "excluded");
        let revert_preview = preview_ownership_group_revert_for_connection(&connection, OwnershipRevertRequest { project_id: 91, target_operation_id: "op-o3-conf-0001".into(), note: String::new() }).unwrap();
        assert_eq!((revert_preview.revertible, revert_preview.conflicted), (confirmed.applied - 1, 1));
        assert_eq!(revert_preview.reasons.get("changed_after_operation"), Some(&1));
        // 预览之后又有变化：拒绝并要求重新预览。
        manual(&mut connection, 1, "related");
        let stale = apply_ownership_group_revert_budgeted(&mut connection, apply(&revert_preview, "op-o3-rv-0002"), None, 500).unwrap_err();
        assert!(stale.starts_with(OWNERSHIP_PREVIEW_STALE), "{stale}");
        let revert_preview = preview_ownership_group_revert_for_connection(&connection, OwnershipRevertRequest { project_id: 91, target_operation_id: "op-o3-conf-0001".into(), note: String::new() }).unwrap();
        let (result, _) = apply_ownership_group_revert_budgeted(&mut connection, apply(&revert_preview, "op-o3-rv-0003"), None, 1).unwrap();
        assert_eq!(result.conflicted, 2);
        assert_eq!(decision(&connection, 2).unwrap().0, "excluded");
        assert_eq!(decision(&connection, 1).unwrap().0, "related");
        let (version, applied_operations, _) = group_state(&connection, run, group);
        assert_eq!((version, applied_operations), (group_before.0 + 2, group_before.1), "confirm +1, revert +1; the group shows no live operation");
        cleanup(root, connection);
    }

    #[test]
    fn revoking_authorization_blocks_downstream_before_the_display_index() {
        let (root, mut connection) = fixture();
        let run = run_analysis(&mut connection, 91, 50);
        let group = group_of(&connection, run, 14).id;
        assert_eq!(exposure_exclusion(&connection, 14), None, "14 is authorized before");
        let before = group_state(&connection, run, group);
        let input = OwnershipRevokeRequest { operation_id: "op-o3-rvk-0001".into(), project_id: 91, run_id: run, group_id: group, note: String::new() };
        // 第一步提交后（模拟此时崩溃）：下游已阻断，展示索引尚未更新。
        let blocked = revoke_ownership_group_authorization_for_connection(&mut connection, input.clone(), true).unwrap();
        assert_eq!((blocked.status.as_str(), blocked.revoked), ("blocked", 1));
        assert_eq!(exposure_exclusion(&connection, 14), Some("authorization_revoked"));
        assert_eq!(group_state(&connection, run, group), before, "display index not yet touched");
        let report = ownership_recover_for_connection(&mut connection).unwrap();
        assert_eq!(report.revocations_completed, vec!["op-o3-rvk-0001".to_string()]);
        assert_eq!(group_state(&connection, run, group), (before.0 + 1, before.1, 1));
        assert_eq!(decision(&connection, 14), Some(("confirmed".into(), "manual".into(), "revoked".into(), 0)));
        assert_eq!(exposure_exclusion(&connection, 14), Some("authorization_revoked"), "recovery never re-authorizes");
        let replay = revoke_ownership_group_authorization_for_connection(&mut connection, input, false).unwrap();
        assert!(replay.replayed);
        assert_eq!((replay.status.as_str(), replay.revoked), ("completed", 1));
        assert_eq!(count(&connection, "SELECT COUNT(*) FROM asset_events WHERE event_type='authorization' AND asset_id=14"), 1);
        let other_project = revoke_ownership_group_authorization_for_connection(&mut connection, OwnershipRevokeRequest { operation_id: "op-o3-rvk-0002".into(), project_id: 92, run_id: run, group_id: group, note: String::new() }, false).unwrap_err();
        assert!(other_project.starts_with(OWNERSHIP_SNAPSHOT_STALE) || other_project.starts_with(OWNERSHIP_REVIEW_INVALID), "{other_project}");
        cleanup(root, connection);
    }

    // ---------------------------------------------------------------- 5. 拆分确认归属 / 授权；§12
    #[test]
    fn confirming_ownership_alone_grants_nothing_and_active_scan_stays_refused() {
        let (root, mut connection) = fixture();
        let downstream = |connection: &rusqlite::Connection| -> (i64, i64, i64) {
            (
                count(connection, "SELECT COUNT(*) FROM sentinel_scans"),
                count(connection, "SELECT COUNT(*) FROM sentinel_targets"),
                count(connection, "SELECT COUNT(*) FROM agent_authorization_controls"),
            )
        };
        let before = downstream(&connection);
        let url = "https://random.example.org/p10";
        let expected = vec![expectation(&connection, 10)];
        let confirmed = confirm_asset_ownership_for_connection(
            &mut connection,
            OwnershipConfirmInput { selections: vec![selection(10)], note: String::new(), domain_rule_scope: String::new() },
            91,
            expected,
        )
        .unwrap();
        assert_eq!((confirmed.changed, confirmed.rule_scope.as_str(), confirmed.learned_rules.len()), (1, "none", 0));
        assert_eq!(decision(&connection, 10), Some(("confirmed".into(), "manual".into(), "unknown".into(), 0)));
        // 归属已确认：下游暴露面授权仍排除它，请求被拒绝。
        assert_eq!(exposure_exclusion(&connection, 10), Some("not_exposure_eligible"));
        let Err(denial) = exposure(&connection).authorize(&reqwest::Url::parse(url).unwrap(), "2026-10-08T00:00:00Z") else {
            panic!("unauthorized exposure request must be denied");
        };
        assert_eq!(denial.code, "host_not_granted");
        // 主动扫描没有被授权：没有任务/目标/授权控制产生，既有主动授权入口拒绝该目标。
        assert_eq!(downstream(&connection), before);
        assert_eq!(authorization_control_setup_in(&connection, &root, "o3-no-such-scan", url).unwrap_err(), "authorization_control_task_binding_invalid");
        // 第二步“授权”走既有受控合同：只放行该 origin 的被动暴露面，不放行兄弟子域，也不产生主动扫描授权。
        let authorized = authorize_asset_ownership_for_connection(&mut connection, OwnershipAuthorizeInput { selections: vec![selection(10)], note: String::new() }, 91, confirmed.expectations.clone()).unwrap();
        assert_eq!(authorized, 1);
        assert_eq!(decision(&connection, 10), Some(("confirmed".into(), "manual".into(), "allowed".into(), 1)));
        assert!(granted(&connection, url));
        assert!(!granted(&connection, "https://sub.random.example.org/"));
        assert_eq!(downstream(&connection), before);
        assert_eq!(authorization_control_setup_in(&connection, &root, "o3-no-such-scan", url).unwrap_err(), "authorization_control_task_binding_invalid");
        cleanup(root, connection);
    }

    #[test]
    fn authorization_requires_a_manual_confirmation_and_rule_scope_is_explicit() {
        let (root, mut connection) = fixture();
        add_asset(&connection, 47, "", "sub.random.example.org");
        add_asset(&connection, 48, "", "github.io");
        let authorize = |connection: &mut rusqlite::Connection, asset: i64, expected: AssetOwnershipDecisionExpectation| {
            authorize_asset_ownership_for_connection(connection, OwnershipAuthorizeInput { selections: vec![selection(asset)], note: String::new() }, 91, vec![expected])
        };
        let confirm = |connection: &mut rusqlite::Connection, asset: i64, scope: &str| {
            let expected = vec![expectation(connection, asset)];
            confirm_asset_ownership_for_connection(connection, OwnershipConfirmInput { selections: vec![selection(asset)], note: String::new(), domain_rule_scope: scope.into() }, 91, expected)
        };
        let current = expectation(&connection, 10);
        let unconfirmed = authorize(&mut connection, 10, current).unwrap_err();
        assert!(unconfirmed.starts_with(OWNERSHIP_AUTHORIZATION_REQUIRES_CONFIRMED), "{unconfirmed}");
        let current = expectation(&connection, 15);
        let automatic = authorize(&mut connection, 15, current).unwrap_err();
        assert!(automatic.starts_with(OWNERSHIP_AUTHORIZATION_REQUIRES_CONFIRMED), "auto attribution is not a confirmation: {automatic}");
        let mut forged = expectation(&connection, 10);
        forged.status = "confirmed".into();
        forged.source = "manual".into();
        let cas = authorize(&mut connection, 10, forged).unwrap_err();
        assert!(cas.starts_with(OWNERSHIP_DECISION_PRECONDITION), "{cas}");
        assert_eq!(decision(&connection, 10), None);
        // 默认精确主机：不覆盖兄弟子域。
        let exact = confirm(&mut connection, 10, "exact_host").unwrap();
        assert_eq!(exact.learned_rules, vec!["host:random.example.org".to_string()]);
        assess_asset_ownership_for_connection(&mut connection, 91).unwrap();
        assert_eq!(decision(&connection, 47).map(|row| row.0), Some("unreviewed".into()));
        // 子域范围必须明确选择。
        let subdomains = confirm(&mut connection, 10, "subdomains").unwrap();
        assert_eq!(subdomains.learned_rules, vec!["domain:random.example.org".to_string()]);
        assess_asset_ownership_for_connection(&mut connection, 91).unwrap();
        assert_eq!(decision(&connection, 47).map(|row| (row.0, row.2)), Some(("attributed".into(), "unknown".into())));
        // 共享域名、公共后缀/平台根、IP、路径范围：拒绝且不写任何结论。
        for (asset, scope) in [(8, "subdomains"), (48, "subdomains"), (9, "subdomains"), (10, "path")] {
            let before = decision(&connection, asset);
            let error = confirm(&mut connection, asset, scope).unwrap_err();
            assert!(error.starts_with(OWNERSHIP_RULE_SCOPE_DENIED), "asset {asset}: {error}");
            assert_eq!(decision(&connection, asset), before, "asset {asset}");
        }
        assert_eq!(count(&connection, "SELECT COUNT(*) FROM asset_ownership_rules WHERE project_id=91"), 2);
        // 旧入口 learnDomainRule 现在也只学精确主机。
        let expected = vec![expectation(&connection, 4)];
        apply_asset_ownership_decisions(&mut connection, AssetOwnershipDecisionInput { selections: vec![selection(4)], status: "confirmed".into(), note: "旧入口".into(), learn_domain_rule: true }, 91, expected).unwrap();
        assert_eq!(count(&connection, "SELECT COUNT(*) FROM asset_ownership_rules WHERE project_id=91 AND rule_type='host' AND pattern='a.zjgov.com.cn'"), 1);
        assert_eq!(count(&connection, "SELECT COUNT(*) FROM asset_ownership_rules WHERE project_id=91 AND rule_type='domain' AND pattern LIKE '%zjgov%'"), 0);
        cleanup(root, connection);
    }

    // ---------------------------------------------------------------- 6. 规模验收（临时库）
    fn rss_kb() -> i64 {
        std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .ok()
            .and_then(|output| String::from_utf8_lossy(&output.stdout).trim().parse().ok())
            .unwrap_or(0)
    }

    fn file_size(path: &Path) -> u64 {
        fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
    }

    struct RssSampler {
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        handle: thread::JoinHandle<i64>,
    }

    impl RssSampler {
        fn start() -> Self {
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let flag = stop.clone();
            let handle = thread::spawn(move || {
                let mut peak = 0;
                while !flag.load(std::sync::atomic::Ordering::Relaxed) {
                    peak = rss_kb().max(peak);
                    thread::sleep(Duration::from_millis(50));
                }
                rss_kb().max(peak)
            });
            Self { stop, handle }
        }

        fn finish(self) -> i64 {
            self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            self.handle.join().unwrap()
        }
    }

    /// 混合数据：省公司/专业子公司/多租户/共享域名/CIDR/重复主机/弱标签/人工结论/删除。
    fn seed_scale(connection: &mut rusqlite::Connection, rows: i64) {
        let provinces = [("浙江", "zj"), ("江苏", "js"), ("广东", "gd"), ("四川", "sc")];
        let transaction = connection.transaction().unwrap();
        {
            let mut asset = transaction.prepare("INSERT INTO assets(id,asset_key,company,host,domain,link,ip,title,last_seen) VALUES(?1,?2,?3,?4,?4,CASE WHEN ?4='' THEN '' ELSE 'https://'||?4||'/x?utm='||?1 END,?5,'门户','2000-01-01 00:00:00')").unwrap();
            let mut member = transaction.prepare("INSERT INTO project_assets(project_id,asset_id,is_deleted,last_seen) VALUES(91,?1,?2,'2000-01-01 00:00:00')").unwrap();
            let mut manual = transaction.prepare("INSERT INTO asset_ownership_decisions(project_id,asset_id,status,confidence,authorization_status,exposure_eligible,source,reason) VALUES(91,?1,'related',70,'unknown',0,'manual','人工')").unwrap();
            for index in 0..rows {
                let id = 1000 + index;
                let (province, code) = provinces[(index % 4) as usize];
                let (company, domain, ip) = match index % 10 {
                    0..=2 => (format!("中国移动通信集团{province}有限公司"), format!("{code}.10086.cn"), String::new()),
                    3 => (format!("中国移动通信集团{province}有限公司"), format!("svc{}.{code}gov.com.cn", index % 7), String::new()),
                    4 => ("中移（苏州）软件技术有限公司".into(), format!("app{}.sz{}.com.cn", index % 3, (index / 10) % 40), String::new()),
                    5 => (format!("161.81.{}.0/22", index % 200), String::new(), format!("161.81.{}.7", index % 200)),
                    6 => (format!("{province}移动"), format!("t{}.github.io", (index / 10) % 25), String::new()),
                    7 => (String::new(), format!("h{}.example{}.org", index, (index / 10) % 30), String::new()),
                    8 => (format!("中国移动通信集团{province}有限公司"), "www.10086.cn".into(), String::new()),
                    _ => (format!("某合作单位{}有限公司", (index / 10) % 50), format!("p{}.partner{}.cn", index, (index / 10) % 50), String::new()),
                };
                asset.execute(params![id, format!("o3-scale-{id}"), company, domain, ip]).unwrap();
                member.execute(params![id, i64::from(index % 97 == 0)]).unwrap();
                if index % 113 == 0 {
                    manual.execute([id]).unwrap();
                }
            }
        }
        transaction.commit().unwrap();
    }

    fn scale_acceptance(rows: i64) {
        let (root, mut connection) = fixture();
        age_all(&connection);
        let db_path = PathBuf::from(connection.path().unwrap());
        let wal_path = PathBuf::from(format!("{}-wal", db_path.display()));
        let seed_started = Instant::now();
        seed_scale(&mut connection, rows);
        let seed_ms = seed_started.elapsed().as_millis();
        connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
        let (db_seeded, wal_seeded) = (file_size(&db_path), file_size(&wal_path));
        let rss_before = rss_kb();
        let sampler = RssSampler::start();
        let started = Instant::now();
        let full = run_mode(&mut connection, "full", OWNERSHIP_ANALYSIS_CHUNK);
        let analysis_ms = started.elapsed().as_millis();
        let rss_peak = sampler.finish();
        let (db_full, wal_full) = (file_size(&db_path), file_size(&wal_path));
        // 完整计数可核验。
        let scope = scope_total(&connection);
        assert!(scope > rows * 9 / 10, "{scope}");
        assert_eq!((full.processed_assets, full.total_assets, full.member_count), (scope, scope, scope));
        assert_eq!(count(&connection, &format!("SELECT COUNT(*) FROM ownership_review_group_members WHERE run_id={}", full.id)), scope);
        assert_eq!(count(&connection, &format!("SELECT COALESCE(SUM(member_count),0) FROM ownership_review_groups WHERE run_id={}", full.id)), scope);
        // 分页：页面不持有全量 ID/URL（每页大小有界，与总行数无关）。
        let (mut cursor, mut page_ms, mut seen, mut member_sum) = (String::new(), Vec::new(), HashSet::new(), 0i64);
        let (mut max_page_bytes, mut max_page_urls) = (0usize, 0usize);
        loop {
            let page_started = Instant::now();
            let page = page_ownership_groups_for_connection(&connection, OwnershipGroupPageInput { project_id: 91, run_id: full.id, page_size: 50, cursor: cursor.clone(), ..Default::default() }).unwrap();
            page_ms.push(page_started.elapsed().as_secs_f64() * 1000.0);
            let json = serde_json::to_string(&page).unwrap();
            max_page_bytes = max_page_bytes.max(json.len());
            max_page_urls = max_page_urls.max(json.matches("https://").count());
            assert_eq!(page.total, full.group_count);
            for item in &page.items {
                assert!(seen.insert(item.id));
                member_sum += item.member_count;
            }
            if page.next_cursor.is_empty() {
                break;
            }
            cursor = page.next_cursor;
        }
        assert_eq!((seen.len() as i64, member_sum), (full.group_count, scope), "every asset counted once");
        assert!(max_page_bytes < 256 * 1024, "{max_page_bytes}");
        assert!(max_page_urls <= 50 * OWNERSHIP_GROUP_SAMPLE_LIMIT, "{max_page_urls}");
        let overview_bytes = serde_json::to_string(&read_ownership_analysis_for_connection(&connection, 91).unwrap()).unwrap().len();
        assert!(overview_bytes < 64 * 1024, "{overview_bytes}");
        let (biggest, biggest_size): (i64, i64) = connection.query_row("SELECT id,member_count FROM ownership_review_groups WHERE run_id=?1 ORDER BY member_count DESC LIMIT 1", [full.id], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
        let (mut cursor, mut member_ms, mut members) = (String::new(), Vec::new(), 0i64);
        loop {
            let member_started = Instant::now();
            let page = page_ownership_group_members_for_connection(&connection, OwnershipMemberPageInput { project_id: 91, run_id: full.id, group_id: biggest, page_size: 100, cursor: cursor.clone() }).unwrap();
            member_ms.push(member_started.elapsed().as_secs_f64() * 1000.0);
            members += page.items.len() as i64;
            assert_eq!(page.total, biggest_size);
            if page.next_cursor.is_empty() {
                break;
            }
            cursor = page.next_cursor;
        }
        assert_eq!(members, biggest_size);
        // 增量：约 1% 行在水位之后被导入器更新。
        connection.execute("UPDATE assets SET company=company||'（更新）',last_seen=datetime('now','localtime') WHERE id>=1000 AND id%100=7", []).unwrap();
        let changed = count(&connection, &format!("SELECT COUNT(*) FROM project_assets pa JOIN assets a ON a.id=pa.asset_id LEFT JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=pa.asset_id WHERE pa.project_id=91 AND a.id>=1000 AND a.id%100=7 AND {OWNERSHIP_SCOPE_SQL}"));
        let started = Instant::now();
        let incremental = run_mode(&mut connection, "auto", OWNERSHIP_ANALYSIS_CHUNK);
        let incremental_ms = started.elapsed().as_millis();
        assert_eq!(incremental.run_kind, "incremental");
        assert_eq!((incremental.reclassified_assets, incremental.carried_assets), (changed, scope - changed));
        assert_eq!(incremental.member_count, scope);
        // 大组写入：一次交互调用只在预算内推进，其余由后台续跑；不向下游发布允许。
        let (group, _): (i64, i64) = connection.query_row("SELECT id,member_count FROM ownership_review_groups WHERE run_id=?1 AND lane='consistent' ORDER BY member_count DESC LIMIT 1", [incremental.id], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
        let started = Instant::now();
        let preview = preview_ownership_group_decision_for_connection(&connection, request(incremental.id, group, "confirm_ownership")).unwrap();
        let preview_ms = started.elapsed().as_millis();
        let eligible_before = eligible_rows(&connection);
        let started = Instant::now();
        let (first, token) = apply_ownership_group_decision_budgeted(&mut connection, apply_input(&preview, "op-o3-scale-0001"), Some(Duration::from_millis(OWNERSHIP_APPLY_IPC_BUDGET_MS)), OWNERSHIP_APPLY_CHUNK).unwrap();
        let first_ms = started.elapsed().as_millis();
        if let Some(token) = token {
            assert_eq!(first.status, "applying");
            assert_eq!(eligible_rows(&connection), eligible_before);
            assert_eq!(ownership_drive_operation(&mut connection, "op-o3-scale-0001", &token, OWNERSHIP_APPLY_CHUNK, None, None).unwrap(), OwnershipStep::Completed);
        }
        let apply_ms = started.elapsed().as_millis();
        let done = read_ownership_operation_for_connection(&connection, 91, "op-o3-scale-0001").unwrap();
        assert_eq!((done.status.as_str(), done.applied, done.failed), ("completed", preview.applicable, 0));
        assert_eq!(op_events(&connection, "op-o3-scale-0001"), preview.applicable);
        assert_eq!(eligible_rows(&connection), eligible_before);
        // 取消延迟：工作线程在跑时请求取消，到状态变为 cancelled 的时间。
        let (run, _) = start_ownership_analysis_with_mode(&mut connection, 91, "full").unwrap();
        let worker_path = db_path.clone();
        let worker = thread::spawn(move || {
            let mut connection = db::open(&worker_path).unwrap();
            let token = ownership_claim_run(&mut connection, run.id).unwrap().unwrap();
            ownership_drive_run(&mut connection, run.id, &token, OWNERSHIP_ANALYSIS_CHUNK, None).unwrap()
        });
        let wait_started = Instant::now();
        while ownership_read_run(&connection, run.id).unwrap().unwrap().processed_assets == 0 && wait_started.elapsed() < Duration::from_secs(30) {
            thread::sleep(Duration::from_millis(2));
        }
        let cancel_started = Instant::now();
        let cancelled = cancel_ownership_analysis_for_connection(&mut connection, 91, run.id);
        let mut cancel_ms = -1i128;
        if cancelled.is_ok() {
            while ownership_read_run(&connection, run.id).unwrap().unwrap().status != "cancelled" && cancel_started.elapsed() < Duration::from_secs(30) {
                thread::sleep(Duration::from_millis(1));
            }
            cancel_ms = cancel_started.elapsed().as_millis() as i128;
        }
        let outcome = worker.join().unwrap();
        let after_cancel = ownership_read_run(&connection, run.id).unwrap().unwrap();
        if cancel_ms >= 0 {
            assert_eq!((outcome, after_cancel.status.as_str()), (OwnershipStep::Cancelled, "cancelled"));
            assert_eq!(count(&connection, &format!("SELECT COUNT(*) FROM ownership_review_group_members WHERE run_id={}", run.id)), 0);
            assert!(after_cancel.processed_assets > 0 && after_cancel.processed_assets <= scope);
        }
        let (db_end, wal_end) = (file_size(&db_path), file_size(&wal_path));
        eprintln!(
            "O3_SCALE rows={rows} scope={scope} groups={} seed_ms={seed_ms} full_analysis_ms={analysis_ms} incremental_ms={incremental_ms} changed={changed} reclassified={} carried={} group_page_p50_ms={:.2} group_page_p95_ms={:.2} group_pages={} max_page_bytes={max_page_bytes} max_page_urls={max_page_urls} overview_bytes={overview_bytes} biggest_group={biggest_size} member_page_p50_ms={:.2} member_page_p95_ms={:.2} member_pages={} large_group_applicable={} preview_ms={preview_ms} apply_first_call_ms={first_ms} apply_first_status={} apply_total_ms={apply_ms} rss_before_kb={rss_before} rss_peak_during_full_kb={rss_peak} db_seeded_bytes={db_seeded} wal_seeded_bytes={wal_seeded} db_after_full_bytes={db_full} wal_after_full_bytes={wal_full} db_end_bytes={db_end} wal_end_bytes={wal_end} cancel_latency_ms={cancel_ms} cancel_processed={}",
            full.group_count,
            incremental.reclassified_assets,
            incremental.carried_assets,
            percentile(&mut page_ms.clone(), 0.5),
            percentile(&mut page_ms.clone(), 0.95),
            page_ms.len(),
            percentile(&mut member_ms.clone(), 0.5),
            percentile(&mut member_ms.clone(), 0.95),
            member_ms.len(),
            preview.applicable,
            first.status,
            after_cancel.processed_assets,
        );
        cleanup(root, connection);
    }

    #[test]
    fn scale_4000_rows_reconciles_and_reports_metrics() {
        scale_acceptance(4000);
    }

    #[test]
    #[ignore]
    fn scale_20000_rows_reconciles_and_reports_metrics() {
        scale_acceptance(20_000);
    }

    #[test]
    #[ignore]
    fn scale_100000_rows_reconciles_and_reports_metrics() {
        scale_acceptance(100_000);
    }
}
