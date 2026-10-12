use super::checkpoint::{self, RunState};
use super::contract::*;
use super::runtime_adapter::{self, BackendReport};
use super::secrets;
use super::store;
use crate::agent_runtime::reducer;
use rusqlite::Connection;
use serde_json::json;
use uuid::Uuid;

fn temp_db(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("oviraptor-rt-{tag}-{}", Uuid::new_v4()));
    let path = crate::db::initialize(&root).unwrap();
    (root, path)
}

fn seeded(connection: &Connection, scan_id: &str) {
    connection
        .execute("INSERT INTO projects(id,name) VALUES(7001,'Runtime')", [])
        .unwrap();
    connection
            .execute(
                "INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,scan_type,attempt_count) VALUES(?1,7001,'Runtime','scanning','测试','web',1)",
                [scan_id],
            )
            .unwrap();
}

fn run_row(scan_id: &str, url: &str) -> store::AgentRunRow {
    store::AgentRunRow::new(
        Uuid::new_v4().to_string(),
        scan_id,
        1,
        url,
        AgentBackendKind::Native,
        AgentRole::Coordinator,
        "plan-hash",
        "evidence-hash",
    )
    .with_budget(100_000, 400_000, 10, 20)
}

#[test]
fn contract_vocabulary_round_trips() {
    assert_eq!(AgentBackendKind::Native.as_str(), "native");
    assert_eq!(AgentBackendKind::parse("  Strix "), None);
    assert_eq!(AgentBackendKind::parse("cursor"), None);
    assert_eq!(ScanMode::parse("").as_str(), "standard");
    assert_eq!(ScanMode::parse("manual_review").as_str(), "manual_review");
    assert!(!ScanMode::Skip.runs_a_backend());
    assert!(ScanMode::Deep.runs_a_backend());
    // Stage 1A §4.2: the retired name still reads, and lands on the role that owns
    // that work now. Writing it out again produces only the new value.
    assert_eq!(
        AgentRole::parse("identity_comparator").as_str(),
        "authorization"
    );
    assert!(AgentRole::Coordinator.may_reduce_terminal());
    assert!(!AgentRole::InputParser.may_reduce_terminal());
    assert_eq!(
        SideEffectClass::parse("controlled_write").as_str(),
        "controlled_write"
    );
    assert_eq!(
        SideEffectClass::parse("anything"),
        SideEffectClass::ReadOnly
    );
    // §11: one terminal vocabulary, and the old spellings still parse so rows
    // written before the change are never read as "unknown".
    assert_eq!(
        TerminalState::parse("partial"),
        Some(TerminalState::Incomplete)
    );
    assert_eq!(
        TerminalState::parse("limited"),
        Some(TerminalState::Limited)
    );
    assert_eq!(
        TerminalState::parse("resume_incompatible"),
        Some(TerminalState::ResumeIncompatible)
    );
    assert_eq!(TerminalState::Incomplete.to_sentinel_status(), "paused");
    assert_eq!(
        TerminalState::BoundedCompleted.to_sentinel_status(),
        "completed_with_gaps"
    );
    assert_eq!(
        TerminalState::Limited.to_sentinel_status(),
        "protected_stop"
    );
    assert_eq!(TerminalState::Completed.to_sentinel_status(), "completed");
    assert_eq!(TerminalState::Failed.to_sentinel_status(), "failed");
    assert_eq!(TerminalState::Cancelled.to_sentinel_status(), "cancelled");
}

/// §11 requirement 4: the new tables must not disturb an existing database.
#[test]
fn migration_is_idempotent_and_keeps_existing_rows() {
    let (root, path) = temp_db("migration");
    {
        let connection = crate::db::open(&path).unwrap();
        seeded(&connection, "scan-migration");
        let row = run_row("scan-migration", "https://a.example.invalid");
        store::create_run(&connection, &row).unwrap();
    }
    // A second initialize on the same file is what an app upgrade does.
    crate::db::initialize(&root).unwrap();
    let connection = crate::db::open(&path).unwrap();
    let runs: i64 = connection
        .query_row("SELECT COUNT(*) FROM agent_runs", [], |row| row.get(0))
        .unwrap();
    let scans: i64 = connection
        .query_row("SELECT COUNT(*) FROM sentinel_scans", [], |row| row.get(0))
        .unwrap();
    assert_eq!((runs, scans), (1, 1));
    for table in [
        "agent_runs",
        "agent_events",
        "agent_messages",
        "tool_invocations",
        "agent_snapshots",
    ] {
        let exists: String = connection
            .query_row(
                "SELECT name FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(exists, table);
    }
}

#[test]
fn events_are_sequenced_unique_and_reject_duplicates() {
    let (_root, path) = temp_db("events");
    let connection = crate::db::open(&path).unwrap();
    seeded(&connection, "scan-events");
    let row = run_row("scan-events", "https://a.example.invalid");
    store::create_run(&connection, &row).unwrap();
    for index in 0..5 {
        let sequence = store::append_event(
            &connection,
            &row.id,
            AgentEventKind::ModelRoundCompleted,
            &json!({"index": index, "progressAdvanced": index % 2 == 0}),
            &["artifact-1".to_string()],
        )
        .unwrap();
        assert_eq!(sequence, index + 1);
    }
    assert_eq!(store::latest_sequence(&connection, &row.id), 5);
    let duplicate = connection.execute(
        "INSERT INTO agent_events(run_id,sequence,event_type) VALUES(?1,3,'plan_frozen')",
        [&row.id],
    );
    assert!(duplicate.unwrap_err().to_string().contains("UNIQUE"));
    // A second run has its own sequence space.
    let other = run_row("scan-events", "https://b.example.invalid");
    store::create_run(&connection, &other).unwrap();
    assert_eq!(
        store::append_event(
            &connection,
            &other.id,
            AgentEventKind::RunCreated,
            &json!({}),
            &[]
        )
        .unwrap(),
        1
    );
}

#[test]
fn run_rows_report_budget_usage_and_terminal_guard() {
    let (_root, path) = temp_db("runs");
    let connection = crate::db::open(&path).unwrap();
    seeded(&connection, "scan-runs");
    let row = run_row("scan-runs", "https://a.example.invalid");
    store::create_run(&connection, &row).unwrap();
    store::set_run_status(&connection, &row.id, AgentRunStatus::Running).unwrap();
    store::settle_usage(
        &connection,
        &row.id,
        &store::UsageDelta {
            input_tokens: 100,
            cached_input_tokens: 40,
            output_tokens: 20,
            total_tokens: 120,
            model_requests: 1,
        },
    )
    .unwrap();
    // A resume re-reports the cumulative total: the row converges on it
    // instead of charging the same spend twice.
    store::settle_usage(
        &connection,
        &row.id,
        &store::UsageDelta {
            input_tokens: 180,
            cached_input_tokens: 60,
            output_tokens: 20,
            total_tokens: 200,
            model_requests: 2,
        },
    )
    .unwrap();
    let loaded = store::load_run(&connection, &row.id).unwrap().unwrap();
    assert_eq!(loaded.used_tokens, 200);
    assert_eq!(loaded.used_cached_tokens, 60);
    assert_eq!(loaded.used_requests, 2);
    assert_eq!(loaded.status, AgentRunStatus::Running);
    let found = store::find_run(
        &connection,
        "scan-runs",
        1,
        "https://a.example.invalid",
        AgentRole::Coordinator,
    )
    .unwrap()
    .unwrap();
    assert_eq!(found.id, row.id);
    assert!(!found.is_terminal());
    assert!(
        store::mark_run_terminal(&connection, &row.id, TerminalState::Completed, "x", "done")
            .unwrap()
    );
    assert!(
        !store::mark_run_terminal(
            &connection,
            &row.id,
            TerminalState::Failed,
            "y",
            "late writer"
        )
        .unwrap(),
        "a terminal run must refuse a second terminal write"
    );
    let after = store::load_run(&connection, &row.id).unwrap().unwrap();
    assert_eq!(after.terminal_state, Some(TerminalState::Completed));
}

#[test]
fn artifact_store_is_content_addressed_and_private() {
    let root = std::env::temp_dir().join(format!("oviraptor-artifacts-{}", Uuid::new_v4()));
    let payload = b"HTTP/1.1 200 OK\r\n\r\n{\"items\":[1]}".to_vec();
    let first = store::store_artifact(&root, &payload).unwrap();
    let second = store::store_artifact(&root, &payload).unwrap();
    assert_eq!(first, second);
    assert_eq!(
        first,
        store::stable_hash(&String::from_utf8_lossy(&payload))
    );
    let path = store::artifact_path(&root, &first);
    assert!(path.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert_eq!(store::read_artifact(&root, &first).unwrap(), payload);
    assert!(store::read_artifact(&root, "missing").is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn secrets_never_reach_events_or_messages() {
    let cookie = "session=abc.def.ghi";
    let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.6-6Sp7A_qFVoXcCwZDmenP";
    let api_key = "sk-AC12bd2F9eA4b1C3D5e6F7a8B9c0D1e2";
    let payload = json!({
        "identity": "session-a",
        "cookie": cookie,
        "headers": [{"name": "Authorization", "value": format!("Bearer {jwt}")}],
        "api_key": api_key,
        "body": format!("request had Cookie: {cookie} and Authorization: Bearer {jwt}"),
        "query": "https://a.example.invalid/u?access_token=opaqueValue123&page=2",
        "owner": "ops@example.com, 13800138000, 110101199003079128",
        "digest": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    });
    let redacted = secrets::redact_json(&payload);
    let text = redacted.to_string();
    for secret in [cookie, jwt, api_key, "opaqueValue123", "ops@example.com"] {
        assert!(
            !text.contains(secret),
            "{secret} survived redaction: {text}"
        );
    }
    assert!(!text.contains("13800138000"));
    assert!(!text.contains("110101199003079128"));
    assert!(text.contains("<redacted:"));
    // The shape of the payload survives so the agent can still reason.
    assert!(text.contains("\"headers\""));
    assert!(text.contains("\"page=2\"") || text.contains("page=2"));
    // A sha256 digest is evidence, not a credential.
    assert!(text.contains("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"));
    let durable = |value: &str| secrets::RedactionContext::durable_marker("auth", value);
    assert_eq!(
        durable(cookie),
        durable(cookie),
        "the same credential must keep a stable marker so audits can correlate"
    );
    assert_ne!(durable(cookie), durable("session=other"));
}

/// §6.2: a model-facing marker must be comparable inside one run but must not
/// let two runs of the same target be linked through their credentials.
#[test]
fn model_markers_are_run_stable_and_cross_run_unlinkable() {
    let token = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.6-6Sp7A_qFVoXcCwZDmenP";
    let run_a = secrets::RedactionContext::new("scan-a/attempt-1");
    let same_run = secrets::RedactionContext::new("scan-a/attempt-1");
    let run_b = secrets::RedactionContext::new("scan-b/attempt-1");
    assert_eq!(
        secrets::redact_text_with(token, Some(&run_a)),
        secrets::redact_text_with(token, Some(&same_run)),
        "the same secret inside one run must stay comparable"
    );
    assert_ne!(
        secrets::redact_text_with(token, Some(&run_a)),
        secrets::redact_text_with(token, Some(&run_b)),
        "two runs must not be linkable through a shared credential"
    );
    let marked = secrets::redact_text_with(token, Some(&run_a));
    assert!(marked.starts_with("<redacted:assertion:"));
    assert!(!marked.contains(token));
}

#[test]
fn single_line_authorization_header_never_keeps_the_original_value() {
    let original = "Authorization: Bearer abc.def.ghi";
    let redacted = secrets::redact_text_with(original, None);
    assert!(redacted.contains("<redacted:auth:"));
    assert!(!redacted.contains("abc.def.ghi"), "{redacted}");
}

#[test]
fn appended_events_are_redacted_at_write_time() {
    let (_root, path) = temp_db("redact");
    let connection = crate::db::open(&path).unwrap();
    seeded(&connection, "scan-redact");
    let row = run_row("scan-redact", "https://a.example.invalid");
    store::create_run(&connection, &row).unwrap();
    let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.6-6Sp7A_qFVoXcCwZDmenP";
    store::append_event(
        &connection,
        &row.id,
        AgentEventKind::ToolInvocationCompleted,
        &json!({"response": {"headers": {"set-cookie": format!("sid={jwt}")}}}),
        &[],
    )
    .unwrap();
    let stored: String = connection
        .query_row(
            "SELECT payload_json FROM agent_events WHERE run_id=?1",
            [&row.id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!stored.contains(jwt));
    assert!(stored.contains("<redacted:"));
}

#[test]
fn mailbox_messages_dedupe_by_key() {
    let (_root, path) = temp_db("mailbox");
    let connection = crate::db::open(&path).unwrap();
    seeded(&connection, "scan-mail");
    let row = run_row("scan-mail", "https://a.example.invalid");
    store::create_run(&connection, &row).unwrap();
    let delivered = store::append_message(
        &connection,
        &row.id,
        "coordinator",
        "contract_verifier",
        AgentMessageKind::Assignment,
        "corr-1",
        "contract:/api/orders",
        &json!({"contractKey": "/api/orders"}),
        &[],
    )
    .unwrap();
    assert!(delivered);
    let duplicate = store::append_message(
        &connection,
        &row.id,
        "coordinator",
        "contract_verifier",
        AgentMessageKind::Assignment,
        "corr-2",
        "contract:/api/orders",
        &json!({"contractKey": "/api/orders"}),
        &[],
    )
    .unwrap();
    assert!(!duplicate, "a replay must not re-deliver the same contract");
    let pending = store::read_undelivered_messages(&connection, &row.id).unwrap();
    assert_eq!(pending.len(), 1);
    store::mark_message_delivered(&connection, "contract:/api/orders").unwrap();
    assert!(store::read_undelivered_messages(&connection, &row.id)
        .unwrap()
        .is_empty());
}

/// Each case starts from the default signal set and flips exactly one input,
/// so the assignment style is deliberate.
#[allow(clippy::field_reassign_with_default)]
#[test]
fn reducer_matrix_matches_the_terminal_state_rules() {
    // cancel beats everything
    let mut signals = TerminalSignals::default();
    signals.cancelled = true;
    signals.protection_signal = Some("WAF".into());
    assert_eq!(reducer::reduce(&signals).state, TerminalState::Cancelled);

    // configuration and integrity are the only hard failures
    let mut signals = TerminalSignals::default();
    signals.configuration_error = Some("未找到可运行的模型配置".into());
    let reduction = reducer::reduce(&signals);
    assert_eq!(reduction.state, TerminalState::Failed);
    assert_eq!(reduction.code, terminal_code::CONFIGURATION);
    let mut signals = TerminalSignals::default();
    signals.configuration_error = Some("Strix 修改了只读证据副本，完整性校验失败".into());
    assert_eq!(
        reducer::reduce(&signals).code,
        terminal_code::EVIDENCE_INTEGRITY
    );

    // protection boundaries
    let mut signals = TerminalSignals::default();
    signals.protection_signal = Some("确认 WAF 或验证码挑战".into());
    assert_eq!(
        reducer::reduce(&signals).code,
        terminal_code::CONFIRMED_CHALLENGE
    );
    let mut signals = TerminalSignals::default();
    signals.protection_signal = Some("HTTP 429 Too Many Requests".into());
    assert_eq!(
        reducer::reduce(&signals).code,
        terminal_code::PERSISTENT_RATE_LIMIT
    );

    // hard ceilings: with evidence they are a bounded completion
    let mut signals = TerminalSignals::default();
    signals.hard_limit_reason = Some("累计 Token 达到硬上限".into());
    signals.evidence_records = 3;
    assert_eq!(
        reducer::reduce(&signals).state,
        TerminalState::BoundedCompleted
    );
    // ...without evidence the work is still pending, never "failed"
    let mut signals = TerminalSignals::default();
    signals.hard_limit_reason = Some("模型调用达到硬上限".into());
    signals.pending_contracts = 4;
    assert_eq!(reducer::reduce(&signals).state, TerminalState::Incomplete);

    // A continuation with an unusable parent checkpoint is its own state: never a
    // failure, never a fresh start (§3.6, §11).
    let resume = TerminalSignals {
        resume_incompatible: Some("续跑状态不兼容，需要重新执行（证据包已变化）".into()),
        evidence_records: 4,
        ..TerminalSignals::default()
    };
    let reduction = reducer::reduce(&resume);
    assert_eq!(reduction.state, TerminalState::ResumeIncompatible);
    assert_eq!(
        reduction.code,
        super::contract::terminal_code::RESUME_INCOMPATIBLE
    );
    assert_eq!(
        TerminalState::ResumeIncompatible.to_sentinel_status(),
        "resume_incompatible"
    );
    // a closed ledger that accounted for every family it owes is a normal
    // completion even with zero findings — "nothing found" is not "nothing done".
    let full = TerminalSignals {
        ledger_closed: true,
        confirmed_findings: 0,
        required_families: vec!["authorization".into()],
        covered_families: vec!["authorization".into()],
        evidence_records: 2,
        ..TerminalSignals::default()
    };
    assert_eq!(reducer::reduce(&full).state, TerminalState::Completed);
    // A closed ledger that still names gaps is a bounded completion, never a plain
    // "completed" that hides the缺口 (§10 Phase 2).
    let gaps = TerminalSignals {
        ledger_closed: true,
        required_families: vec!["authorization".into(), "business_flow".into()],
        covered_families: vec!["authorization".into()],
        evidence_records: 2,
        ..TerminalSignals::default()
    };
    assert_eq!(
        reducer::reduce(&gaps).state,
        TerminalState::BoundedCompleted
    );
    // …and with no executed evidence at all it stays recoverable work.
    let no_evidence = TerminalSignals {
        ledger_closed: true,
        required_families: vec!["authorization".into()],
        ..TerminalSignals::default()
    };
    assert_eq!(
        reducer::reduce(&no_evidence).state,
        TerminalState::Incomplete
    );

    // Incomplete is never produced because nothing was found
    let signals = TerminalSignals {
        pending_contracts: 0,
        confirmed_findings: 0,
        evidence_records: 9,
        ..TerminalSignals::default()
    };
    assert_eq!(reducer::reduce(&signals).state, TerminalState::Completed);
    let signals = TerminalSignals {
        pending_contracts: 2,
        ..TerminalSignals::default()
    };
    assert_eq!(reducer::reduce(&signals).state, TerminalState::Incomplete);

    // unsupported capability stays recoverable
    let signals = TerminalSignals {
        unsupported_capability: Some("unsupported_capability:tools".into()),
        ..TerminalSignals::default()
    };
    assert_eq!(
        reducer::reduce(&signals).code,
        terminal_code::UNSUPPORTED_CAPABILITY
    );
    assert_eq!(reducer::reduce(&signals).state, TerminalState::Incomplete);
}

#[test]
fn reducer_is_a_single_writer_per_run() {
    let (_root, path) = temp_db("reducer");
    let connection = crate::db::open(&path).unwrap();
    seeded(&connection, "scan-reduce");
    let row = run_row("scan-reduce", "https://a.example.invalid");
    store::create_run(&connection, &row).unwrap();
    let first = reducer::commit(
        &connection,
        &row.id,
        &TerminalSignals {
            ledger_closed: true,
            required_families: vec!["authorization".into()],
            covered_families: vec!["authorization".into()],
            evidence_records: 1,
            detail: "收口".into(),
            ..TerminalSignals::default()
        },
    )
    .unwrap();
    assert_eq!(first.state, TerminalState::Completed);
    let second = reducer::commit(
        &connection,
        &row.id,
        &TerminalSignals {
            configuration_error: Some("late failure".into()),
            ..TerminalSignals::default()
        },
    )
    .unwrap();
    assert_eq!(
        second.state,
        TerminalState::Completed,
        "a later report must not rewrite the terminal state"
    );
    let events: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_events WHERE run_id=?1 AND event_type='terminal_reduced'",
            [&row.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(events, 1);
}

#[test]
fn replay_from_snapshot_never_double_charges_budget() {
    let (_root, path) = temp_db("replay");
    let connection = crate::db::open(&path).unwrap();
    seeded(&connection, "scan-replay");
    let row = run_row("scan-replay", "https://a.example.invalid");
    store::create_run(&connection, &row).unwrap();
    let mut state = RunState::new(row.id.clone(), vec!["contract:/api/a".into()]);
    for _ in 0..6 {
        store::append_event(
                &connection,
                &row.id,
                AgentEventKind::ModelRoundCompleted,
                &json!({
                    "usage": {"inputTokens": 100, "cachedInputTokens": 20, "outputTokens": 10, "totalTokens": 110},
                    "progressAdvanced": true
                }),
                &[],
            )
            .unwrap();
        state.turns += 1;
        state.model_requests += 1;
        state.input_tokens += 100;
        state.cached_input_tokens += 20;
        state.output_tokens += 10;
        state.used_tokens += 110;
    }
    checkpoint::write_checkpoint(&connection, &state).unwrap();
    // Events after the snapshot boundary.
    for _ in 0..2 {
        store::append_event(
                &connection,
                &row.id,
                AgentEventKind::ModelRoundCompleted,
                &json!({
                    "usage": {"inputTokens": 50, "cachedInputTokens": 0, "outputTokens": 5, "totalTokens": 55},
                    "progressAdvanced": false
                }),
                &[],
            )
            .unwrap();
    }
    store::append_event(
        &connection,
        &row.id,
        AgentEventKind::ProtectionDetected,
        &json!({"signal": "Cloudflare Ray ID"}),
        &[],
    )
    .unwrap();
    let recovered = checkpoint::recover(&connection, &row.id).unwrap();
    assert_eq!(
        recovered.events_replayed, 3,
        "only events after the snapshot replay"
    );
    assert_eq!(recovered.state.turns, 8);
    assert_eq!(recovered.state.used_tokens, 6 * 110 + 2 * 55);
    assert_eq!(recovered.state.no_progress_streak, 2);
    assert_eq!(
        recovered.state.protection_signal.as_deref(),
        Some("Cloudflare Ray ID")
    );
    // Replaying the same log twice from the snapshot is stable.
    let again = checkpoint::recover(&connection, &row.id).unwrap();
    assert_eq!(again.state, recovered.state);
    assert_eq!(recovered.state.input_tokens, 6 * 100 + 2 * 50);
    assert_eq!(recovered.state.cached_input_tokens, 6 * 20);
    assert_eq!(recovered.state.uncached_tokens(), 580 + 70);
}

#[test]
fn recovery_interrupts_unfinished_read_only_calls_and_requeues_their_contracts() {
    let (_root, path) = temp_db("interrupt");
    let connection = crate::db::open(&path).unwrap();
    seeded(&connection, "scan-int");
    let row = run_row("scan-int", "https://a.example.invalid");
    store::create_run(&connection, &row).unwrap();
    let invocation = store::begin_tool_invocation(
        &connection,
        &row.id,
        "inv-1-001",
        "inspect_evidence",
        1,
        "contract:/api/orders",
        "session-a",
        &json!({"url": "https://a.example.invalid/api/orders"}),
        "allow",
    )
    .unwrap();
    store::append_event(
        &connection,
        &row.id,
        AgentEventKind::ToolInvocationCompleted,
        &json!({"contractKey": "contract:/api/orders", "progressSignature": "e1"}),
        &[],
    )
    .unwrap();
    let mut state = RunState::new(row.id.clone(), Vec::new());
    state.completed_contracts = vec!["contract:/api/orders".into()];
    state.progress_signature = "e1".into();
    checkpoint::write_checkpoint(&connection, &state).unwrap();
    // The process dies before `finish_tool_invocation` can run.
    let recovered = checkpoint::recover(&connection, &row.id).unwrap();
    assert_eq!(recovered.interrupted_invocations, 1);
    assert!(
        recovered.state.completed_contracts.is_empty(),
        "an interrupted call has no determined result: {:?}",
        recovered.state.completed_contracts
    );
    assert_eq!(
        recovered.state.pending_contracts,
        vec!["contract:/api/orders"]
    );
    let again = checkpoint::recover(&connection, &row.id).unwrap();
    assert_eq!(
        again.state.pending_contracts,
        recovered.state.pending_contracts
    );
    assert_eq!(
        again.state.completed_contracts,
        recovered.state.completed_contracts
    );
    let status: String = connection
        .query_row(
            "SELECT status FROM tool_invocations WHERE id=?1",
            [invocation],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "interrupted");
}

#[test]
fn recovery_refuses_ambiguous_target_calls_even_after_a_previous_failed_recovery() {
    let (_root, path) = temp_db("ambiguous-target-recovery");
    let connection = crate::db::open(&path).unwrap();
    seeded(&connection, "scan-ambiguous");
    let row = run_row("scan-ambiguous", "https://a.example.invalid");
    store::create_run(&connection, &row).unwrap();
    let invocation = store::begin_tool_invocation(
        &connection,
        &row.id,
        "inv-1-001",
        "replay_http",
        1,
        "contract:/api/orders",
        "anonymous",
        &json!({"method": "POST", "url": "https://a.example.invalid/api/orders"}),
        "allow",
    )
    .unwrap();
    for _ in 0..2 {
        assert_eq!(
            checkpoint::recover(&connection, &row.id)
                .err()
                .expect("must fail closed"),
            "interrupted_tool_result_unknown_manual_reconciliation_required"
        );
    }
    let status: String = connection
        .query_row(
            "SELECT status FROM tool_invocations WHERE id=?1",
            [invocation],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "interrupted");
}

#[test]
fn tool_invocation_rows_record_policy_and_progress() {
    let (_root, path) = temp_db("invocations");
    let connection = crate::db::open(&path).unwrap();
    seeded(&connection, "scan-tool");
    let row = run_row("scan-tool", "https://a.example.invalid");
    store::create_run(&connection, &row).unwrap();
    let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.6-6Sp7A_qFVoXcCwZDmenP";
    let invocation = store::begin_tool_invocation(
        &connection,
        &row.id,
        "inv-1-002",
        "identity.compare",
        1,
        "contract:/api/me",
        "session-a",
        &json!({"headers": {"authorization": format!("Bearer {jwt}")}}),
        "allow",
    )
    .unwrap();
    let stored: String = connection
        .query_row(
            "SELECT input_summary_json FROM tool_invocations WHERE id=?1",
            [invocation],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        !stored.contains(jwt),
        "credential material must never be stored"
    );
    store::finish_tool_invocation(
        &connection,
        invocation,
        "confirmed",
        "e1:r1",
        "artifact-9",
        "",
    )
    .unwrap();
    let row_state: (String, String) = connection
        .query_row(
            "SELECT status,progress_signature FROM tool_invocations WHERE id=?1",
            [invocation],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(row_state, ("confirmed".into(), "e1:r1".into()));
    assert!(store::contract_without_result(&connection, &row.id)
        .unwrap()
        .is_empty());
    let open = store::begin_tool_invocation(
        &connection,
        &row.id,
        "inv-1-001",
        "http.replay",
        1,
        "contract:/api/pending",
        "anonymous",
        &json!({}),
        "allow",
    )
    .unwrap();
    assert_eq!(
        store::contract_without_result(&connection, &row.id)
            .unwrap()
            .len(),
        1
    );
    store::finish_tool_invocation(&connection, open, "insufficient_evidence", "", "", "").unwrap();
    assert!(store::contract_without_result(&connection, &row.id)
        .unwrap()
        .is_empty());
}

#[test]
fn runtime_adapter_persists_execution_stops_even_when_checkpoint_claims_resumable() {
    for code in [
        terminal_code::REQUEST_RECONCILIATION_REQUIRED,
        terminal_code::EXECUTION_AUTHORIZATION_DENIED,
    ] {
        let (_root, path) = temp_db("adapter-execution-stop");
        let connection = crate::db::open(&path).unwrap();
        seeded(&connection, "scan-stop");
        let mut report = BackendReport::new(
            "scan-stop",
            1,
            "https://a.example.invalid",
            AgentBackendKind::Native,
        );
        report.resumable = true;
        report.ledger_closed = true;
        report.evidence_records = 10;
        report.required_families = Vec::new();
        report.hard_limit_reason = Some("request budget".into());
        if code == terminal_code::REQUEST_RECONCILIATION_REQUIRED {
            report.request_reconciliation_required = Some("manual reconciliation required".into());
        } else {
            report.execution_authorization_denied = Some("execution grant revoked".into());
        }
        let run_id = runtime_adapter::open_run(&connection, &report).unwrap();
        let reduction = runtime_adapter::close_run(&connection, &run_id, &report)
            .unwrap()
            .expect("execution stop must not remain automatically resumable");
        assert_eq!(reduction.state, TerminalState::Incomplete);
        assert_eq!(reduction.code, code);
        let row = store::load_run(&connection, &run_id).unwrap().unwrap();
        assert!(row.is_terminal());
        assert_eq!(row.terminal_code, code);
        let recovered = checkpoint::recover(&connection, &run_id).unwrap();
        assert_eq!(recovered.state.terminal, Some(TerminalState::Incomplete));
        assert_eq!(recovered.state.terminal_code, code);
        // Neither a later completed report nor a replay may erase this boundary.
        let later = reducer::commit(&connection, &run_id, &TerminalSignals::default()).unwrap();
        assert_eq!(later.code, code);
    }
}

#[test]
fn terminal_signals_accept_historical_json_without_execution_stop_fields() {
    let mut historical = serde_json::to_value(TerminalSignals::default()).unwrap();
    historical
        .as_object_mut()
        .unwrap()
        .remove("requestReconciliationRequired");
    historical
        .as_object_mut()
        .unwrap()
        .remove("executionAuthorizationDenied");
    let restored: TerminalSignals = serde_json::from_value(historical).unwrap();
    assert!(restored.request_reconciliation_required.is_none());
    assert!(restored.execution_authorization_denied.is_none());
}

#[test]
fn runtime_adapter_reports_facts_and_lets_the_reducer_own_the_state() {
    let (_root, path) = temp_db("adapter");
    let connection = crate::db::open(&path).unwrap();
    seeded(&connection, "scan-adapter");
    let mut report = BackendReport::new(
        "scan-adapter",
        1,
        "https://a.example.invalid",
        AgentBackendKind::Native,
    );
    report.plan_json = json!({"backend": "native", "mode": "standard"});
    report.evidence_records = 4;
    report.covered_families = vec!["authorization".into()];
    report.hard_limit_reason = Some("累计上下文 Token 达到绝对上限".into());
    let run_id = runtime_adapter::open_run(&connection, &report).unwrap();
    let reduction = runtime_adapter::close_run(&connection, &run_id, &report).unwrap();
    let reduction = reduction.expect("a settled report must reduce");
    assert_eq!(reduction.state, TerminalState::BoundedCompleted);
    // While a run is still open, re-registering the same attempt reuses it.
    let open_report = BackendReport::new(
        "scan-adapter",
        2,
        "https://b.example.invalid",
        AgentBackendKind::Native,
    );
    let first = runtime_adapter::open_run(&connection, &open_report).unwrap();
    assert_eq!(
        runtime_adapter::open_run(&connection, &open_report).unwrap(),
        first
    );
    // Once terminal, that run is history: the next attempt gets its own row
    // instead of having its state rewritten.
    assert!(store::load_run(&connection, &run_id)
        .unwrap()
        .unwrap()
        .is_terminal());
    let next_attempt = runtime_adapter::open_run(&connection, &report).unwrap();
    assert_ne!(next_attempt, run_id);
    let runs: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_runs WHERE scan_id=?1",
            ["scan-adapter"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(runs, 3);
    let plan_events: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_events WHERE run_id=?1 AND event_type='plan_frozen'",
            [&run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(plan_events, 1);
    let snapshot: String = connection
        .query_row(
            "SELECT snapshot_json FROM agent_snapshots WHERE run_id=?1",
            [&run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(snapshot.contains("bounded_completed") || snapshot.contains("completed"));
}

#[test]
fn runtime_adapter_cancelled_run_is_not_reduced_to_failure() {
    let (_root, path) = temp_db("adapter-cancel");
    let connection = crate::db::open(&path).unwrap();
    seeded(&connection, "scan-cancel");
    let mut report = BackendReport::new(
        "scan-cancel",
        1,
        "https://a.example.invalid",
        AgentBackendKind::Native,
    );
    report.cancelled = true;
    report.configuration_error = Some("配置错误".into());
    let run_id = runtime_adapter::open_run(&connection, &report).unwrap();
    let reduction = runtime_adapter::close_run(&connection, &run_id, &report).unwrap();
    let reduction = reduction.expect("a settled report must reduce");
    assert_eq!(reduction.state, TerminalState::Cancelled);
    let status: String = connection
        .query_row(
            "SELECT terminal_state FROM agent_runs WHERE id=?1",
            [&run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "cancelled");
}

/// Requirement: no runtime table may hold credential material, so the
/// schema itself must not offer a column for it.
#[test]
fn agent_tables_carry_no_credential_columns() {
    let (_root, path) = temp_db("columns");
    let connection = crate::db::open(&path).unwrap();
    seeded(&connection, "scan-columns");
    let row = run_row("scan-columns", "https://a.example.invalid");
    store::create_run(&connection, &row).unwrap();
    // Exact column names: `used_tokens` and the budget columns are counters,
    // not credential storage.
    let credential_columns = [
        "cookie",
        "set_cookie",
        "authorization",
        "api_key",
        "apikey",
        "x_api_key",
        "password",
        "passwd",
        "secret",
        "client_secret",
        "jwt",
        "access_token",
        "refresh_token",
        "session_token",
        "private_key",
    ];
    for table in [
        "agent_runs",
        "agent_events",
        "agent_messages",
        "tool_invocations",
        "agent_snapshots",
    ] {
        let mut statement = connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        let columns = statement
            .query_map([], |inner| inner.get::<_, String>(1))
            .unwrap()
            .flatten()
            .collect::<Vec<_>>();
        assert!(!columns.is_empty(), "{table} missing");
        for column in &columns {
            let lowered = column.to_ascii_lowercase();
            assert!(
                !credential_columns.contains(&lowered.as_str()),
                "{table}.{column} must not store credentials"
            );
        }
    }
    let loaded = store::load_run(&connection, &row.id).unwrap().unwrap();
    assert_eq!(loaded.backend, AgentBackendKind::Native);
    assert_eq!(loaded.role, AgentRole::Coordinator);
    assert_eq!(loaded.status, AgentRunStatus::Prepared);
    assert_eq!(loaded.hard_token_budget, 400_000);
    assert_eq!(loaded.hard_request_budget, 20);
}

// Stage 1A: the multi-agent contract and persistence skeleton.
include!("stage1a_tests.rs");

#[path = "tests_backend_retirement.rs"]
mod backend_retirement;


#[test]
fn readonly_bootstrap_role_sets_keep_input_parser_and_business_logic_on_floor() {
    assert!(AgentRole::InputParser.is_readonly_bootstrap_role());
    assert!(AgentRole::BusinessLogic.requires_web_reviewer_floor_on_bootstrap());
    assert!(!AgentRole::Upload.is_readonly_bootstrap_role());
    assert!(!AgentRole::Concurrency.requires_web_reviewer_floor_on_bootstrap());
    assert_eq!(
        AgentRole::InputParser.specialist_readonly_binding_lane(),
        Some("read_only_analysis")
    );
    assert!(AgentRole::SpaApiMapper.is_readonly_bootstrap_role());
    assert!(!AgentRole::SpaApiMapper.requires_web_reviewer_floor_on_bootstrap());
    assert!(AgentRole::InputParser.allows_undispatched_worker_replacement());
    assert!(AgentRole::BusinessLogic.allows_undispatched_worker_replacement());
    assert!(AgentRole::ClientSide.allows_undispatched_worker_replacement());
    assert!(AgentRole::ExternalSurface.allows_undispatched_worker_replacement());
    assert!(!AgentRole::Upload.allows_undispatched_worker_replacement());
    assert!(!AgentRole::EvidenceReviewer.allows_undispatched_worker_replacement());
    assert!(AgentRole::InputParser.allows_supervised_readonly_bootstrap());
    assert!(AgentRole::BusinessLogic.allows_supervised_readonly_bootstrap());
    assert!(AgentRole::ClientSide.allows_supervised_readonly_bootstrap());
    assert!(!AgentRole::DeepInvestigator.allows_supervised_readonly_bootstrap());
    assert!(!AgentRole::RepoMapper.allows_supervised_readonly_bootstrap());
}
