#[test]
fn native_process_observer_real_analyzer_entry_persists_while_actual_tool_waits() {
    // Both real production routes: host process and container::run/start --attach.
    // The executable fixture supplies tool output, never an observer or journal.
    for pinned_image in [false, true] {
        assert_analyzer_entry_streams(pinned_image, false);
    }
}

#[test]
fn native_process_replay_real_analyzer_host_and_container_entry_is_readable_before_child_exit() {
    for pinned_image in [false, true] {
        assert_analyzer_entry_streams(pinned_image, true);
    }
}

fn assert_analyzer_entry_streams(pinned_image: bool, via_ipc: bool) {
    use crate::native_pipeline::analyzer::{self, AnalyzerEngine, AnalyzerSpec};
    use std::{cell::Cell, os::unix::fs::PermissionsExt};
    let f = Fixture::new();
    let connection = Connection::open(&f.db).unwrap();
    connection.execute_batch("CREATE TABLE analyzer_container_receipts(
        receipt_id TEXT PRIMARY KEY,invocation_key TEXT,scan_id TEXT,attempt_number INTEGER,
        purpose TEXT,container_name TEXT,container_id TEXT DEFAULT '',owner_token TEXT,
        create_args_json TEXT,cleanup_status TEXT DEFAULT 'pending',detail TEXT DEFAULT '',
        updated_at TEXT);
        CREATE TABLE analyzer_runs(invocation_key TEXT PRIMARY KEY,scan_id TEXT,attempt_number INTEGER,
        engine TEXT,status TEXT,gap_code TEXT,version TEXT,rule_pack_digest TEXT,image_digest TEXT,
        network_disabled INTEGER,repository_read_only INTEGER,sarif_path TEXT,stdout_truncated INTEGER,
        duration_millis INTEGER,args_json TEXT,evidence_json TEXT,created_at TEXT);").unwrap();
    let scratch = f.root.join("analyzer-output");
    fs::create_dir(&scratch).unwrap();
    let program = f.root.join("fixture-analyzer.sh");
    fs::write(&program,r#"#!/bin/sh
id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
run_step() {
    printf 'native-analyzer-stream-line\n'
    printf 'password=source-path-secret\n' >&2
    while [ ! -f gate ]; do sleep 0.01; done
    case "$out" in /out/*) out="./${out#/out/}";; esac
    printf '%s' '{"version":"2.1.0","runs":[{"tool":{"driver":{"name":"fixture"}},"results":[]}]}' > "$out"
    printf ended > ended
}
case "$1" in
    --version) printf 'fixture-version\n';;
    create)
        prev=''
        for arg in "$@"; do
            [ "$prev" = '--name' ] && printf '%s' "$arg" > container.name
            [ "$prev" = '--label' ] && printf '%s' "${arg#*=}" > container.owner
            prev="$arg"
        done
        printf '%s\n' "$@" > create.args
        printf exists > container.exists
        printf '%s\n' "$id"
        ;;
    start)
        prev=''; out=''
        while IFS= read -r arg; do
            [ "$prev" = '--output' ] && out="$arg"
            prev="$arg"
        done < create.args
        if [ "$prev" = '--version' ]; then printf 'fixture-version\n'; else run_step; fi
        ;;
    container)
        case "$2" in
            inspect)
                [ -f container.exists ] || exit 1
                name=$(cat container.name); owner=$(cat container.owner)
                printf '[{"Id":"%s","Name":"/%s","Config":{"Labels":{"io.oviraptor.native-owner":"%s"}}}]\n' "$id" "$name" "$owner"
                ;;
            rm) rm -f container.exists;;
            ls) [ -f container.exists ] && printf '%s\n' "$id";;
        esac
        ;;
    *)
        out=''
        while [ "$#" -gt 0 ]; do
            if [ "$1" = '--output' ]; then shift; out="$1"; fi
            shift
        done
        run_step
        ;;
esac
exit 0
"#).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    let rules = f.root.join("rules.yml");
    fs::write(&rules, b"rules: []").unwrap();
    let spec = AnalyzerSpec {
        engine: AnalyzerEngine::Semgrep,
        program,
        image: pinned_image.then(|| format!("fixture/analyzer@sha256:{}", "a".repeat(64))),
        rule_pack: rules,
        languages: vec!["python".into()],
        repository: f.root.clone(),
        scratch_dir: scratch.clone(),
        scan_id: f.scope.scan_id.clone(),
        attempt_number: 1,
        limits: ProcessLimits {
            timeout: Duration::from_millis(900),
            ..Default::default()
        },
    };
    let witnessed = Cell::new(false);
    let outcome = analyzer::run_logged(&connection, &spec, &|| {
        // The production entry owns the sink. Read its actual SQLite rows while
        // the actual tool/start --attach subprocess is still awaiting the gate.
        let reader = Connection::open_with_flags(&f.db, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let found: bool = reader
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM native_process_log_rows r
            JOIN native_process_log_executions e ON e.execution_id=r.execution_id
            WHERE e.scan_id='scan-a' AND e.attempt_number=1 AND e.stage='semgrep:step-0'
            AND e.branch='source' AND e.dispatch_claim_id='11111111-1111-4111-8111-111111111111'
            AND r.message='native-analyzer-stream-line')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let found = if via_ipc {
            // Same business read helper called by read_native_process_log IPC;
            // actual production analyzer owns writer and process observer.
            let view = crate::scanning::process_log_test_support::read_native_process_log_snapshot(
                &f.db, "scan-a", 1, None, None, 0, 300,
            )
            .unwrap();
            let committed = serde_json::to_value(view).unwrap();
            assert_eq!(committed["scanId"], "scan-a");
            assert_eq!(committed["attempt"], 1);
            committed["rows"].as_array().unwrap().iter().any(|row| {
                row["message"] == "native-analyzer-stream-line"
                    && row["stage"] == "semgrep:step-0"
                    && row["branch"] == "source"
                    && row["dispatchClaimId"] == "11111111-1111-4111-8111-111111111111"
            })
        } else {
            found
        };
        if found && !scratch.join("ended").exists() {
            witnessed.set(true);
            fs::write(scratch.join("gate"), b"release").unwrap();
        }
        false
    })
    .unwrap();
    assert!(
        witnessed.get(),
        "actual analyzer output not persisted before process exit; pinned={pinned_image}"
    );
    assert!(outcome.produced_results());
    let texts: String = connection
        .prepare("SELECT message FROM native_process_log_rows ORDER BY sequence")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join("\n");
    assert!(!texts.contains("source-path-secret"));
    assert!(texts.contains("<redacted:"));
    for stage in ["semgrep:version", "semgrep:step-0"] {
        let count:i64=connection.query_row("SELECT COUNT(*) FROM native_process_log_executions
            WHERE scan_id='scan-a' AND attempt_number=1 AND stage=?1 AND invocation_key=?2 AND state='completed'
            AND branch='source' AND dispatch_claim_id='11111111-1111-4111-8111-111111111111'",
            rusqlite::params![stage,outcome.invocation_key],|row|row.get(0)).unwrap();
        assert_eq!(count, 1);
    }
    if pinned_image {
        let cleaned: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM analyzer_container_receipts
            WHERE scan_id='scan-a' AND attempt_number=1 AND cleanup_status='confirmed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cleaned, 2);
        assert!(!scratch.join("container.exists").exists());
    }
    // Logging does not mutate admission or require any Root tables.
    let claims: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM native_branch_dispatches
        WHERE scan_id='scan-a' AND attempt_number=1 AND branch='source'
        AND claim_id='11111111-1111-4111-8111-111111111111' AND claimed_at='fixture-claimed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(claims, 1);
}
