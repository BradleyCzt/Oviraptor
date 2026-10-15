/// Phase 0 §11 收尾：`terminal_state` 空表示"还没有终态"，但一个读不懂的非空值不能
/// 被读成同一件事——那会让已收口的 run 看起来还可以继续。
#[test]
fn an_unrecognised_terminal_state_is_a_read_error() {
    let (root, connection) = stage1a_connection("stage1a-terminal");
    let run = run_row("stage1a-scan", "https://app.example.invalid");
    store::create_run(&connection, &run).unwrap();
    assert!(store::load_run(&connection, &run.id)
        .unwrap()
        .unwrap()
        .terminal_state
        .is_none());
    connection
        .execute(
            "UPDATE agent_runs SET terminal_state='completed_with_gaps' WHERE id=?1",
            [&run.id],
        )
        .unwrap();
    assert_eq!(
        store::load_run(&connection, &run.id)
            .unwrap()
            .unwrap()
            .terminal_state,
        Some(TerminalState::BoundedCompleted)
    );
    connection
        .execute(
            "UPDATE agent_runs SET terminal_state='maybe_completed' WHERE id=?1",
            [&run.id],
        )
        .unwrap();
    let error = store::load_run(&connection, &run.id).unwrap_err();
    assert!(error.contains("terminal_state") && error.contains("agent_runs"), "{error}");
    assert!(error.contains(&run.id), "错误必须点出是哪一行：{error}");
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

/// §3.3 收尾：事件是恢复重放的唯一来源，损坏的 payload 或 artifact 引用必须让整次
/// 重放失败，而不是被当作"没有内容"继续跑下去。
#[test]
fn corrupted_event_columns_fail_the_replay_reader() {
    let (root, connection) = stage1a_connection("stage1a-events");
    let run = run_row("stage1a-scan", "https://app.example.invalid");
    store::create_run(&connection, &run).unwrap();
    store::append_event(
        &connection,
        &run.id,
        AgentEventKind::EvidenceProduced,
        &json!({"evidenceId": "ev-1"}),
        &["artifact-1".to_string()],
    )
    .unwrap();
    assert_eq!(store::read_events_after(&connection, &run.id, 0).unwrap().len(), 1);
    let valid_payload: String = connection
        .query_row("SELECT payload_json FROM agent_events WHERE run_id=?1", [&run.id], |row| row.get(0))
        .unwrap();
    let valid_refs: String = connection
        .query_row("SELECT artifact_refs_json FROM agent_events WHERE run_id=?1", [&run.id], |row| row.get(0))
        .unwrap();
    for (column, broken) in [
        ("payload_json", "{\"unterminated"),
        ("artifact_refs_json", "[\"unclosed"),
    ] {
        let restored = (&valid_payload, &valid_refs);
        connection
            .execute(
                &format!("UPDATE agent_events SET {column}=?1 WHERE run_id=?2"),
                rusqlite::params![broken, run.id],
            )
            .unwrap();
        let error = store::read_events_after(&connection, &run.id, 0).unwrap_err();
        assert!(
            error.contains(column) && error.contains("agent_events"),
            "{column} 损坏时应报出表与列：{error}"
        );
        let (payload, refs) = restored;
        connection
            .execute(
                "UPDATE agent_events SET payload_json=?1, artifact_refs_json=?2 WHERE run_id=?3",
                rusqlite::params![payload, refs, run.id],
            )
            .unwrap();
        assert_eq!(store::read_events_after(&connection, &run.id, 0).unwrap().len(), 1);
    }
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

/// §5 收尾：真实升级路径。把库退回 Stage 1A 之前的形状（四张表、九个新列都不在），
/// 再走一次 `db::initialize`，历史行必须还在并按单 Agent 默认读回，索引和触发器重建，
/// 并且整个过程可以重跑。
#[test]
fn a_legacy_database_is_upgraded_in_place_and_stays_readable() {
    let (root, path) = temp_db("stage1a-legacy-upgrade");
    let run_id;
    {
        let connection = crate::db::open(&path).unwrap();
        seeded(&connection, "legacy-scan");
        let run = run_row("legacy-scan", "https://app.example.invalid");
        store::create_run(&connection, &run).unwrap();
        run_id = run.id.clone();
        // This post-Stage1 trigger references the tables/columns removed below;
        // a genuine pre-Stage1 database did not contain it either.
        connection.execute_batch("DROP TRIGGER IF EXISTS agent_directive_proposal_state_guard_v2").unwrap();
        // Same for the open paid Root retention guard (it reads root_run_id).
        connection.execute_batch("DROP TRIGGER IF EXISTS root_paid_open_run_retained").unwrap();
        // The later worker-report event trigger joins agent_assignments, which
        // did not exist in a pre-Stage1 database either.
        connection.execute_batch("DROP TRIGGER IF EXISTS agent_collaboration_homogeneous_report").unwrap();
        for table in [
            "agent_review_decisions",
            "agent_evidence_edges",
            "agent_evidence_nodes",
            "agent_assignments",
        ] {
            connection
                .execute_batch(&format!("DROP TABLE IF EXISTS {table}"))
                .unwrap();
        }
        for column in [
            "root_run_id",
            "assignment_id",
            "lane",
            "orchestration_policy",
            "capability_lease_json",
            "reserved_tokens",
            "reserved_requests",
            "heartbeat_at",
            "cancel_requested_at",
        ] {
            connection
                .execute(&format!("ALTER TABLE agent_runs DROP COLUMN {column}"), [])
                .unwrap();
        }
    }
    crate::db::initialize(&root).unwrap();
    let connection = crate::db::open(&path).unwrap();
    let restored = store::load_run(&connection, &run_id).unwrap().unwrap();
    assert_eq!(restored.id, run_id, "升级不得丢掉历史 run");
    assert_eq!(restored.orchestration_policy, MultiAgentPolicy::Single);
    assert_eq!(restored.lane, None);
    assert_eq!(restored.root_run_id, "");
    assert_eq!(restored.capability_lease, Vec::<String>::new());
    assert_eq!(restored.reserved_tokens, 0);
    let objects: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('agent_assignments','agent_evidence_nodes','agent_evidence_edges','agent_review_decisions')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(objects, 4, "四张 Stage 1A 表必须重建");
    let guards: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' AND name IN ('agent_evidence_edges_endpoints_insert','agent_evidence_edges_endpoints_update')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(guards, 2, "端点隔离触发器必须重建");
    let retention: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' AND name='root_paid_open_run_retained'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retention, 1, "升级后必须恢复开放付费 Root 保留守卫");
    let report_events: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' AND name='agent_collaboration_homogeneous_report'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(report_events, 1, "升级后必须恢复 worker report 事件触发器");
    drop(connection);
    crate::db::initialize(&root).unwrap();
    let connection = crate::db::open(&path).unwrap();
    assert_eq!(
        store::load_run(&connection, &run_id).unwrap().unwrap().id,
        run_id,
        "第二次升级不得改动历史行"
    );
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

/// §5.4 作为数据库规则：绕过仓储的裸 SQL 也不能把两个任务或两个 revision 的证据串在
/// 一起，也不得留下悬空边。
#[test]
fn the_schema_refuses_edges_outside_their_endpoints_root_and_revision() {
    use super::evidence_graph::contract::{EvidenceNodeKind, EvidenceProvenance};
    use super::evidence_graph::store::insert_evidence_node;
    let (root, connection) = stage1a_connection("stage1a-edge-trigger");
    create_runs(&connection, &["root-a", "root-b"]);
    for (id, root_run, revision) in
        [("t-a1", "root-a", 1), ("t-b1", "root-b", 1), ("t-a2", "root-a", 2)]
    {
        insert_evidence_node(
            &connection,
            &evidence_node(
                id,
                root_run,
                revision,
                EvidenceNodeKind::Endpoint,
                EvidenceProvenance::Observed,
                json!({"id": id}),
            ),
        )
        .unwrap();
    }
    for (name, to, edge_root, edge_revision) in [
        ("跨 root", "t-b1", "root-a", 1),
        ("跨 revision", "t-a2", "root-a", 1),
        ("悬空端点", "t-gone", "root-a", 1),
    ] {
        let error = connection
            .execute(
                &format!(
                    "INSERT INTO agent_evidence_edges(root_run_id,revision,from_node_id,to_node_id,kind,created_by_run_id) VALUES('{edge_root}',{edge_revision},'t-a1','{to}','supports','root-a')"
                ),
                [],
            )
            .unwrap_err();
        assert!(
            error.to_string().contains("evidence") || error.to_string().contains("FOREIGN KEY"),
            "{name} 必须被拒绝：{error}"
        );
    }
    let rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM agent_evidence_edges", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0, "被拒绝的边不得留下行");
    connection
        .execute(
            "INSERT INTO agent_evidence_edges(root_run_id,revision,from_node_id,to_node_id,kind,created_by_run_id) VALUES('root-a',1,'t-a1','t-a1','supports','root-a')",
            [],
        )
        .expect("同 root 同 revision 的边必须可写");
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}
