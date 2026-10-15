/// One cancellation token for the complete native frontend attempt.
#[derive(Clone)]
struct NativeReconControl {
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    deadline: Instant,
    log_owner: Option<NativeHelperLogBinding>,
}

impl NativeReconControl {
    fn new(timeout: Duration) -> Self {
        Self {
            cancelled: Default::default(),
            deadline: Instant::now() + timeout,
            log_owner: None,
        }
    }

    fn new_logged(timeout:Duration,path:&Path,scan_id:&str,attempt:i64,target:&str)->Result<Self,String> {
        let mut control=Self::new(timeout);
        control.log_owner=Some(NativeHelperLogBinding::load(path,scan_id,attempt,target)?);
        Ok(control)
    }
    fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
    }

    fn check(&self) -> Result<(), String> {
        if self.cancelled.load(std::sync::atomic::Ordering::Acquire) {
            Err("前端侦察已取消；不会发布晚到结果".into())
        } else if Instant::now() >= self.deadline {
            Err("前端侦察已达到本轮时限；不会发布晚到结果".into())
        } else {
            Ok(())
        }
    }
}

fn read_native_http_body(
    mut response: reqwest::blocking::Response,
    limit: usize,
) -> Result<String, String> {
    let mut bytes = Vec::new();
    Read::by_ref(&mut response)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > limit {
        return Err(format!("响应体超过 {limit} 字节，未解析截断内容"));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// How the Node helper itself stopped. The exit code and signal are kept apart
/// because a kill-by-signal run has no code at all.
fn native_helper_exit_label(status: &std::process::ExitStatus) -> String {
    #[cfg(unix)]
    let signal = status.signal();
    #[cfg(not(unix))]
    let signal: Option<i32> = None;
    if let Some(signal) = signal {
        return format!("辅助程序被信号 {signal} 终止");
    }
    match status.code() {
        Some(code) => format!("辅助程序退出码 {code}"),
        None => "辅助程序退出状态未知".to_string(),
    }
}

/// Helper stderr is a local diagnostic, not evidence: it is capped and any
/// credential-shaped material is masked before it can be stored or displayed.
fn native_helper_stderr_note(error_output: &[u8]) -> String {
    let text = String::from_utf8_lossy(error_output);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "辅助程序未输出 stderr".to_string();
    }
    bounded_redacted_text(trimmed, 1200)
}

/// The shipped JavaScript is part of the application release, not a mutable
/// tool installed from the host PATH. Compare against bytes compiled into this
/// release; a missing or modified resource is an unavailable capability.
/// This is integrity checking, not a process/network sandbox or a signature
/// for separately downloaded capability bundles.
fn verify_bundled_worker(helper: &Path) -> Result<(), String> {
    fn verify_file(path: &Path, expected: &[u8], missing: &str, mismatch: &str) -> Result<(), String> {
        let metadata = fs::symlink_metadata(path).map_err(|_| missing.to_string())?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(mismatch.into());
        }
        let actual = fs::read(path).map_err(|_| mismatch.to_string())?;
        if Sha256::digest(&actual) != Sha256::digest(expected) {
            return Err(mismatch.into());
        }
        Ok(())
    }

    match helper.file_name().and_then(|name| name.to_str()) {
        Some("9_frontend_runtime_probe.cjs") => {
            verify_file(helper, include_bytes!("../../resources/workers/9_frontend_runtime_probe.cjs"),
                "bundled_worker_missing", "bundled_worker_digest_mismatch")?;
            let config = helper.parent().and_then(Path::parent)
                .ok_or("bundled_worker_browser_locations_missing")?.join("config/browser-locations.json");
            verify_file(&config, include_bytes!("../../resources/config/browser-locations.json"),
                "bundled_worker_browser_locations_missing", "bundled_worker_browser_locations_mismatch")
        },
        Some("8_js_ast_analyzer.cjs") => {
            verify_file(
                helper,
                include_bytes!("../../resources/workers/8_js_ast_analyzer.cjs"),
                "bundled_worker_missing",
                "bundled_worker_digest_mismatch",
            )?;
            verify_file(
                &helper.with_file_name("babel-parser.cjs"),
                include_bytes!("../../resources/workers/babel-parser.cjs"),
                "bundled_worker_parser_missing",
                "bundled_worker_parser_digest_mismatch",
            )
        }
        _ => Err("bundled_worker_unregistered".into()),
    }
}

// Both old JSON API and diagnostics use this actual private capture path.
fn run_native_json_helper(helper:&Path,payload:&JsonValue,runtime_path:&OsString,
 proxy:Option<&str>,no_proxy:&str,timeout:Duration,control:&NativeReconControl)->Result<JsonValue,String>{
 let output=run_native_json_helper_raw(helper,payload,runtime_path,proxy,no_proxy,timeout,control)?;
 serde_json::from_slice(&output).map_err(|error|format!("{} JSON 无效：{error}",helper.display()))
}

#[cfg(test)]
mod native_helper_tests {
    use super::*;

    #[test]
    fn bundled_worker_integrity_rejects_missing_tampered_and_unregistered_files() {
        let directory = std::env::temp_dir().join(format!("oviraptor-worker-integrity-{}", Uuid::new_v4()));
        fs::create_dir_all(directory.join("workers")).unwrap();
        fs::create_dir(directory.join("config")).unwrap();
        let runtime = directory.join("workers/9_frontend_runtime_probe.cjs");
        fs::write(&runtime, include_bytes!("../../resources/workers/9_frontend_runtime_probe.cjs")).unwrap();
        let locations = directory.join("config/browser-locations.json");
        assert_eq!(verify_bundled_worker(&runtime).unwrap_err(), "bundled_worker_browser_locations_missing");
        fs::write(&locations, b"{}").unwrap();
        assert_eq!(verify_bundled_worker(&runtime).unwrap_err(), "bundled_worker_browser_locations_mismatch");
        fs::write(&locations, include_bytes!("../../resources/config/browser-locations.json")).unwrap();
        assert!(verify_bundled_worker(&runtime).is_ok());
        fs::write(&runtime, b"changed worker").unwrap();
        assert_eq!(verify_bundled_worker(&runtime).unwrap_err(), "bundled_worker_digest_mismatch");
        let unknown = directory.join("unknown.cjs");
        fs::write(&unknown, b"console.log('unregistered')").unwrap();
        assert_eq!(verify_bundled_worker(&unknown).unwrap_err(), "bundled_worker_unregistered");
        fs::remove_file(&runtime).unwrap();
        assert_eq!(verify_bundled_worker(&runtime).unwrap_err(), "bundled_worker_missing");
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn ast_worker_requires_the_pinned_parser_in_the_same_bundle() {
        let directory = std::env::temp_dir().join(format!("oviraptor-ast-integrity-{}", Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let worker = directory.join("8_js_ast_analyzer.cjs");
        let parser = directory.join("babel-parser.cjs");
        fs::write(&worker, include_bytes!("../../resources/workers/8_js_ast_analyzer.cjs")).unwrap();
        assert_eq!(verify_bundled_worker(&worker).unwrap_err(), "bundled_worker_parser_missing");
        fs::write(&parser, b"host parser substitution").unwrap();
        assert_eq!(verify_bundled_worker(&worker).unwrap_err(), "bundled_worker_parser_digest_mismatch");
        fs::write(&parser, include_bytes!("../../resources/workers/babel-parser.cjs")).unwrap();
        assert!(verify_bundled_worker(&worker).is_ok());
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn tampered_runtime_worker_never_spawns_node() {
        let directory = std::env::temp_dir().join(format!("oviraptor-runtime-integrity-{}", Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let worker = directory.join("9_frontend_runtime_probe.cjs");
        fs::write(&worker, b"process.stdout.write(JSON.stringify({available:true}))").unwrap();
        let control = NativeReconControl::new(Duration::from_secs(5));
        let error = run_native_json_helper(
            &worker,
            &serde_json::json!({}),
            &std::env::var_os("PATH").unwrap_or_default(),
            None,
            "",
            Duration::from_secs(5),
            &control,
        ).unwrap_err();
        assert_eq!(error, "bundled_worker_digest_mismatch");
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn resolve_helper_node_prefers_modern_over_usr_local_v16() {
        #[cfg(unix)]
        let directory = std::env::temp_dir().join(format!("oviraptor-node-resolver-{}", Uuid::new_v4()));
        #[cfg(unix)]
        let (runtime, legacy, modern) = {
            use std::os::unix::fs::PermissionsExt;
            let legacy_dir = directory.join("usr local bin");
            let modern_dir = directory.join("homebrew bin");
            fs::create_dir_all(&legacy_dir).unwrap();
            fs::create_dir_all(&modern_dir).unwrap();
            let legacy = legacy_dir.join("node");
            let modern = modern_dir.join("node");
            for (path, version) in [(&legacy, "v16.0.0"), (&modern, "v22.0.0")] {
                fs::write(
                    path,
                    format!("#!/bin/sh\n[ \"$#\" -eq 1 ] && [ \"$1\" = \"-v\" ] || exit 64\nprintf '%s\\n' '{version}'\n"),
                ).unwrap();
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
            }
            // Put the old executable first, as the former GUI PATH did.
            let runtime = std::env::join_paths([&legacy_dir, &modern_dir]).unwrap();
            (runtime, legacy, modern)
        };
        #[cfg(not(unix))]
        let runtime = {
            // Native Windows executables cannot be replaced by POSIX scripts;
            // keep the real resolver/version check using the application's PATH.
            let home = std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            sentinel_runtime_path(&home)
        };
        let chosen = resolve_helper_node(&runtime).expect("modern node");
        let version = std::process::Command::new(&chosen)
            .arg("-v")
            .output()
            .expect("node -v");
        assert!(version.status.success(), "node -v failed for {}", chosen.display());
        let text = String::from_utf8_lossy(&version.stdout);
        let major = text
            .trim()
            .trim_start_matches('v')
            .split('.')
            .next()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);
        let minimum_major = if cfg!(unix) { 22 } else { 18 };
        assert!(major >= minimum_major, "expected Node {minimum_major}+, got {text} from {}", chosen.display());
        #[cfg(unix)]
        {
            assert_ne!(chosen, legacy, "Node 16 must not win over modern Node");
            // The resolver also considers explicit overrides and installed paths;
            // a newer host Node may legitimately beat the v22 fixture.
            if chosen == modern {
                assert_eq!(text.trim(), "v22.0.0");
            }
            fs::remove_dir_all(directory).unwrap();
        }
    }

    fn helper(
        source: &str,
        timeout: Duration,
        control: &NativeReconControl,
    ) -> Result<JsonValue, String> {
        let directory =
            std::env::temp_dir().join(format!("oviraptor-helper-test-{}", Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("fixture.cjs");
        fs::write(&path, source).unwrap();
        let result = run_native_json_helper(
            &path,
            &serde_json::json!({"fixture":true}),
            &std::env::var_os("PATH").unwrap_or_default(),
            None,
            "",
            timeout,
            control,
        );
        fs::remove_dir_all(&directory).unwrap();
        result
    }

    #[test]
    fn large_stdout_and_stderr_do_not_deadlock() {
        let control = NativeReconControl::new(Duration::from_secs(10));
        let value = helper("require('fs').readFileSync(0); process.stderr.write('w'.repeat(200000)); process.stdout.write(JSON.stringify({body:'x'.repeat(300000)}));",
            Duration::from_secs(5), &control).unwrap();
        assert_eq!(value["body"].as_str().unwrap().len(), 300000);
    }

    #[test]
    fn cancelled_helper_stops_promptly() {
        let control = NativeReconControl::new(Duration::from_secs(10));
        let cancellation = control.clone();
        let trigger = thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            cancellation.cancel();
        });
        let started = Instant::now();
        let error = helper(
            "require('fs').readFileSync(0); setInterval(()=>{},1000);",
            Duration::from_secs(8),
            &control,
        )
        .unwrap_err();
        trigger.join().unwrap();
        assert!(error.contains("取消"));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn malformed_json_is_not_accepted_as_success() {
        let control = NativeReconControl::new(Duration::from_secs(10));
        let error = helper(
            "require('fs').readFileSync(0); process.stdout.write('{broken');",
            Duration::from_secs(5),
            &control,
        )
        .unwrap_err();
        assert!(error.contains("JSON 无效"));
    }

    #[test]
    fn helper_stderr_note_redacts_secrets_and_caps_at_1200_chars() {
        let secret = "Authorization: Bearer super-secret-token-value";
        let filler = "z".repeat(4000);
        let note = native_helper_stderr_note(format!("{secret}\n{filler}").as_bytes());
        assert!(
            note.chars().count() <= 1200,
            "stderr note must stay within the 1200-char store/display budget, got {}",
            note.chars().count()
        );
        assert!(
            !note.contains("super-secret-token-value"),
            "raw bearer token must not survive redaction: {note}"
        );
        assert!(
            note.contains("<redacted:auth:") || note.to_ascii_lowercase().contains("redacted"),
            "credential-shaped material must be marked redacted: {note}"
        );
    }

    #[test]
    fn empty_helper_stderr_gets_an_explicit_placeholder() {
        assert_eq!(
            native_helper_stderr_note(b"   \n\t  "),
            "辅助程序未输出 stderr"
        );
    }

    #[test]
    fn failed_helper_exit_error_string_keeps_redacted_stderr() {
        let control = NativeReconControl::new(Duration::from_secs(10));
        // Prefer the token= shape the shared redactor always rewrites; Bearer
        // headers can leave a trailing literal after the auth marker.
        let error = helper(
            "require('fs').readFileSync(0); process.stderr.write('token=super-secret-token-value boom'); process.exit(7);",
            Duration::from_secs(5),
            &control,
        )
        .unwrap_err();
        assert!(
            error.contains("退出码 7"),
            "expected exit code 7 label, got: {error}"
        );
        assert!(error.contains("辅助程序 stderr："), "{error}");
        assert!(
            !error.contains("super-secret-token-value"),
            "failed-exit stderr must be redacted before it reaches the Err string: {error}"
        );
        assert!(
            error.contains("<redacted:auth:") || error.to_ascii_lowercase().contains("redacted"),
            "redaction marker missing: {error}"
        );
    }
}

#[cfg(test)]
include!("native_helper_test_capture.rs");
