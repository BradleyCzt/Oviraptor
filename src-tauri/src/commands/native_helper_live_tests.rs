include!("native_helper_live_fixture.rs");
include!("native_helper_live_limits.rs");
#[test]
fn native_helper_live_actual_ast_and_browser_entries_commit_stderr_while_child_waits() {
    for browser in [false, true] {
        let f = HelperFixture::new();
        let control = f.control();
        let worker=f.worker(if browser {"fake_runtime_probe.cjs"}else{"fixture.cjs"},r#"
   const fs=require('fs'),path=require('path');fs.readFileSync(0);
   process.stderr.write('helper-live-stage\n');
   const timer=setInterval(()=>{if(fs.existsSync(path.join(__dirname,'gate'))){clearInterval(timer);
    fs.writeFileSync(path.join(__dirname,'ended'),'finished');process.stdout.write('{"available":true,"apis":[],"raw":"unchanged"}');}},5);
  "#);
        let db = f.db.clone();
        let root = f.root.clone();
        let claim = f.claim.clone();
        let watcher = thread::spawn(move || {
            let start = Instant::now();
            let mut witnessed = false;
            while start.elapsed() < Duration::from_secs(2) {
                let view =
                    read_native_process_log_snapshot(&db, "web-a", 1, None, None, 0, 300).unwrap();
                let page = serde_json::to_value(view).unwrap();
                if page["rows"].as_array().unwrap().iter().any(|row| {
                    row["message"] == "helper-live-stage"
                        && row["branch"] == "web"
                        && row["dispatchClaimId"] == claim
                }) && !root.join("ended").exists()
                {
                    witnessed = true;
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            fs::write(root.join("gate"), b"release").unwrap();
            witnessed
        });
        let result = if browser {
            native_runtime_probe(
                &worker,
                "about:blank",
                None,
                1,
                1,
                "local",
                None,
                "",
                &runtime(),
                &[],
                false,
                None,
                &control,
            )
        } else {
            run_ast_helper(
                &worker,
                "about:blank",
                "const path='/catalog';",
                &runtime(),
                &control,
            )
        };
        let before_exit = watcher.join().unwrap();
        assert!(result.is_ok(), "{browser}: {result:?}");
        assert!(before_exit,"production AST/browser entry delivered no committed live diagnostic before EOF: browser={browser}");
        let rows = f.rows();
        assert!(rows
            .iter()
            .all(|r| r.hint.scope.branch == "web" && r.hint.scope.attempt == 1));
        assert!(rows
            .iter()
            .any(|r| r.hint.stream == "stderr" && r.hint.stream_sequence == 1));
        assert_eq!(f.native(), "{\"bytes\":\"original-full-source-contract\"}");
    }
}

#[test]
fn native_helper_live_raw_json_bytes_stay_exact_while_cross_chunk_stderr_secrets_are_redacted() {
    let f = HelperFixture::new();
    let control = f.control();
    let raw = "{\n  \"password\":\"native-raw-secret\",\"body\":\"\\u5b8c\\u6574\"\n}\n";
    let source = format!(
        r#"require('fs').readFileSync(0);process.stderr.write('Authoriz');
 setTimeout(()=>process.stderr.write('ation: Be'),15);
 setTimeout(()=>process.stderr.write('arer helper-bearer-secret\npassword=helper-'),30);
 setTimeout(()=>process.stderr.write('password-secret\napi_key=helper-api-secret\n'),45);
 setTimeout(()=>process.stdout.write({}),60);"#,
        serde_json::to_string(raw).unwrap()
    );
    let worker = f.worker("fixture.cjs", &source);
    let output = run_native_json_helper_raw(
        &worker,
        &json!({}),
        &runtime(),
        None,
        "",
        Duration::from_secs(2),
        &control,
    )
    .unwrap();
    assert_eq!(
        output,
        raw.as_bytes(),
        "machine stdout bytes changed during live diagnostic capture"
    );
    let rows = f.rows();
    let text = rows
        .iter()
        .map(|r| r.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !rows.is_empty(),
        "stderr was never persisted by the actual helper runner"
    );
    for secret in [
        "native-raw-secret",
        "helper-bearer-secret",
        "helper-password-secret",
        "helper-api-secret",
    ] {
        assert!(!text.contains(secret), "helper persisted secret: {secret}");
    }
    assert_eq!(text.matches("<redacted:").count(), 3);
    assert!(rows.iter().all(|r| r.hint.stream != "stdout"));
    assert_helper_capture_limits_and_queue_gap();
}

#[test]
fn native_helper_live_cancel_strong_kills_actual_descendant_and_fences_late_logs() {
    let f = HelperFixture::new();
    let control = f.control();
    let cancelled = control.clone();
    let worker=f.worker("fixture.cjs",r#"
  const fs=require('fs'),cp=require('child_process'),path=require('path');fs.readFileSync(0);
  const code="const fs=require('fs'),p=process.argv[1];fs.writeFileSync(p+'/descendant-started','yes');setTimeout(()=>fs.writeFileSync(p+'/escaped','late'),1400);setInterval(()=>{},1000);";
  cp.spawn(process.execPath,['-e',code,__dirname],{stdio:'inherit'});
  const ready=setInterval(()=>{if(fs.existsSync(path.join(__dirname,'descendant-started'))){clearInterval(ready);process.stderr.write('descendant-is-running\n');}},5);
  setInterval(()=>{},1000);
 "#);
    let db = f.db.clone();
    // The precondition (helper + descendant cold-started and their diagnostic
    // committed) takes ~0.5-0.7 s alone in a debug build and longer when the
    // suite runs in parallel, so a fixed 800 ms window flaked. Cancel as soon
    // as it is observed, bounded well inside the helper's 3 s timeout, and
    // judge cancellation promptness from the cancel instant itself.
    let trigger = thread::spawn(move || {
        let start = Instant::now();
        let mut observed = false;
        while start.elapsed() < Duration::from_secs(2) {
            let page = serde_json::to_value(
                read_native_process_log_snapshot(&db, "web-a", 1, None, None, 0, 300).unwrap(),
            )
            .unwrap();
            if page["rows"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["message"] == "descendant-is-running")
            {
                observed = true;
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let cancelled_at = Instant::now();
        cancelled.cancel();
        (observed, start.elapsed(), cancelled_at)
    });
    let error = run_native_json_helper(
        &worker,
        &json!({}),
        &runtime(),
        None,
        "",
        Duration::from_secs(3),
        &control,
    )
    .unwrap_err();
    let returned_at = Instant::now();
    let (observed, waited, cancelled_at) = trigger.join().unwrap();
    assert!(
        observed,
        "no live committed diagnostic while actual descendant ran (waited {waited:?}, descendant started: {})",
        f.root.join("descendant-started").exists()
    );
    assert!(
        error.contains("取消"),
        "cancellation reason was lost: {error}"
    );
    // Stricter than the former whole-run `< 2 s` (which left >= 1.2 s for the
    // cancel itself after the 800 ms window): strong cancel must return < 1 s.
    let cancel_latency = returned_at.saturating_duration_since(cancelled_at);
    assert!(
        cancel_latency < Duration::from_secs(1),
        "strong cancel took {cancel_latency:?} to return"
    );
    assert!(f.root.join("descendant-started").exists());
    let before = serde_json::to_string(&f.rows()).unwrap();
    thread::sleep(Duration::from_millis(1500));
    assert!(
        !f.root.join("escaped").exists(),
        "actual descendant escaped strong kill"
    );
    assert_eq!(
        serde_json::to_string(&f.rows()).unwrap(),
        before,
        "helper appended late logs after return"
    );
}

#[test]
fn native_helper_live_normal_exit_reaps_pipe_inheriting_descendant_without_accepting_late_effect() {
    let f = HelperFixture::new();
    let control = f.control();
    let worker=f.worker("fixture.cjs",r#"
  const fs=require('fs'),cp=require('child_process'),path=require('path');fs.readFileSync(0);
  const code="const fs=require('fs'),p=process.argv[1];fs.writeFileSync(p+'/descendant-started','yes');setTimeout(()=>{fs.writeFileSync(p+'/escaped','late');process.exit(0);},700);";
  cp.spawn(process.execPath,['-e',code,__dirname],{stdio:'inherit'});
  const ready=setInterval(()=>{if(fs.existsSync(path.join(__dirname,'descendant-started'))){clearInterval(ready);process.stderr.write('safe-eof-tail',()=>process.stdout.write('{"result":"native"}',()=>process.exit(0)));}},5);
 "#);
    let started = Instant::now();
    let result = run_native_json_helper(
        &worker,
        &json!({}),
        &runtime(),
        None,
        "",
        Duration::from_secs(2),
        &control,
    )
    .unwrap();
    assert_eq!(result["result"], "native");
    assert!(f.root.join("descendant-started").exists());
    assert!(started.elapsed() < Duration::from_secs(2));
    thread::sleep(Duration::from_millis(800));
    assert!(
        !f.root.join("escaped").exists(),
        "normally exited helper left a live inherited-pipe descendant"
    );
    assert!(f.rows().iter().any(|r| r.message == "safe-eof-tail"));
}

#[test]
fn native_helper_live_shipped_ast_and_invalid_browser_emit_safe_stages_without_http() {
    let f = HelperFixture::new();
    let control = f.control();
    let workers = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/workers");
    let ast = run_ast_helper(
        &workers.join("8_js_ast_analyzer.cjs"),
        "about:blank",
        "const url='/catalog';",
        &runtime(),
        &control,
    )
    .unwrap();
    assert!(ast["apis"].is_array());
    // Invalid target exits before browser launch/CDP/HTTP. Real shipped byte gate,
    // helper process and logger still run, rather than replacing worker functions.
    let browser = native_runtime_probe(
        &workers.join("9_frontend_runtime_probe.cjs"),
        "",
        None,
        1,
        1,
        "local",
        None,
        "",
        &runtime(),
        &[],
        false,
        None,
        &control,
    )
    .unwrap();
    assert!(browser["errors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e == "invalid_target_url"));
    let rows = f.rows();
    for (stage, text) in [
        ("node_ast:parse", "native_ast:startup"),
        ("node_ast:parse", "native_ast:parse"),
        ("node_ast:parse", "native_ast:complete"),
        ("node_browser:probe", "native_browser:startup"),
        ("node_browser:probe", "native_browser:result_ready"),
    ] {
        assert!(
            rows.iter()
                .any(|r| r.hint.scope.stage == stage && r.message == text),
            "real bundled stage missing: {stage}/{text}"
        );
    }
    assert_eq!(
        browser["probeStage"], "startup",
        "diagnostic stages changed original Native stdout metadata"
    );
    assert!(rows
        .iter()
        .all(|r| !r.message.contains("about:blank") && !r.message.contains("/catalog")));
}

#[test]
fn native_helper_live_missing_unclaimed_source_or_replaced_attempt_never_mints_web_log_owner() {
    for sql in [
        "UPDATE native_branch_dispatches SET claim_id='',claimed_at=''",
        "UPDATE native_scan_branches SET status='failed';",
        "UPDATE sentinel_scans SET attempt_count=2",
        "INSERT INTO sentinel_deleted_scans(scan_id) VALUES('web-a')",
    ] {
        let f = HelperFixture::new();
        let c = rusqlite::Connection::open(&f.db).unwrap();
        c.execute_batch(sql).unwrap();
        assert!(
            NativeReconControl::new_logged(
                Duration::from_secs(1),
                &f.db,
                "web-a",
                1,
                "about:blank"
            )
            .is_err(),
            "log owner minted from wrong scope: {sql}"
        );
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM native_process_log_executions",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(f.native(), "{\"bytes\":\"original-full-source-contract\"}");
    }
    let source = HelperFixture::new_for("source");
    assert!(
        NativeReconControl::new_logged(
            Duration::from_secs(1),
            &source.db,
            "web-a",
            1,
            "about:blank"
        )
        .is_err(),
        "actual Source claim must never become Web log authority"
    );
    let f = HelperFixture::new();
    let missing = f.root.join("missing.sqlite3");
    assert!(NativeReconControl::new_logged(
        Duration::from_secs(1),
        &missing,
        "web-a",
        1,
        "about:blank"
    )
    .is_err());
    assert!(!missing.exists());
}
