use super::host_process::{self, ProcessLimits};
use std::{path::Path, time::Duration};

#[test]
fn host_pre_cancel_and_zero_deadline_do_not_launch() {
    let missing = Path::new("/nonexistent/execution-service-must-not-launch");
    let cancelled = host_process::run(missing, &[], None, &ProcessLimits::default(), &|| true)
        .expect("cancellation must win before executable lookup");
    assert!(cancelled.cancelled);
    assert!(!cancelled.timed_out);
    assert!(!cancelled.succeeded());

    let timed_out = host_process::run(
        missing,
        &[],
        None,
        &ProcessLimits {
            timeout: Duration::ZERO,
            ..ProcessLimits::default()
        },
        &|| false,
    )
    .expect("an exhausted deadline must win before executable lookup");
    assert!(timed_out.timed_out);
    assert!(!timed_out.cancelled);
    assert!(!timed_out.succeeded());
}

#[test]
fn diagnostic_event_preserves_native_journal_type() {
    let event = super::OutputEvent {
        stream: "stderr",
        stream_sequence: 7,
        message: "redacted diagnostic".into(),
        gap: false,
    };
    // The existing journal takes this exact type, with no lossy translation or
    // invented task-state/progress event at the mechanism boundary.
    let journal_event: crate::native_pipeline::process::log::Event = event;
    assert_eq!(journal_event.stream, "stderr");
    assert_eq!(journal_event.stream_sequence, 7);
    assert_eq!(journal_event.message, "redacted diagnostic");
    assert!(!journal_event.gap);
}

#[cfg(not(target_os = "macos"))]
#[test]
fn offline_parser_adapter_retains_unsupported_platform_without_host_fallback() {
    let request = crate::tool_supply::container_parser::Request {
        code: "def analyze(data):\n return data".into(),
        input: serde_json::json!({"fixture": true}),
        limits: crate::tool_supply::container_parser::Limits {
            memory_mib: 32,
            timeout_seconds: 1,
            output_bytes: 256,
        },
        runtime_digest: "fixture-unavailable-runtime".into(),
    };
    let error = super::offline_parser::execute(&request, &|| false).unwrap_err();
    assert_eq!(
        error,
        "unsupported_sandbox:container_parser_platform_unverified"
    );
}

#[cfg(unix)]
mod host_compatibility {
    use super::*;
    use std::{fs, path::PathBuf};

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "oviraptor-execution-service-{}",
                uuid::Uuid::new_v4()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn limits() -> ProcessLimits {
        ProcessLimits {
            timeout: Duration::from_secs(3),
            ..ProcessLimits::default()
        }
    }

    #[test]
    fn host_arguments_stay_data_and_optional_cwd_keeps_existing_semantics() {
        let scratch = Scratch::new();
        let argument = "$(touch escaped); quoted ' argument with spaces";
        let args = vec![
            "-c".into(),
            "printf '%s' \"$1\"; printf '|'; pwd -P".into(),
            "fixture".into(),
            argument.into(),
        ];
        let in_scratch = host_process::run(
            Path::new("/bin/sh"),
            &args,
            Some(&scratch.0),
            &limits(),
            &|| false,
        )
        .unwrap();
        assert!(in_scratch.succeeded());
        assert_eq!(
            in_scratch.stdout_text(),
            format!(
                "{argument}|{}\n",
                scratch.0.canonicalize().unwrap().display()
            )
        );
        assert!(!scratch.0.join("escaped").exists());

        let inherited = host_process::run(
            Path::new("/bin/sh"),
            &["-c".into(), "pwd -P".into()],
            None,
            &limits(),
            &|| false,
        )
        .unwrap();
        assert!(inherited.succeeded());
        assert_eq!(
            inherited.stdout_text(),
            format!(
                "{}\n",
                std::env::current_dir()
                    .unwrap()
                    .canonicalize()
                    .unwrap()
                    .display()
            )
        );
    }

    #[test]
    fn host_environment_remains_inherited_and_is_not_a_sandbox_claim() {
        // Read an existing environment value without mutating global test state.
        let path = std::env::var_os("PATH").expect("the test runner has a PATH");
        let run = host_process::run(
            Path::new("/bin/sh"),
            &["-c".into(), "printf '%s' \"$PATH\"".into()],
            None,
            &limits(),
            &|| false,
        )
        .unwrap();
        assert!(run.succeeded());
        use std::os::unix::ffi::OsStrExt;
        assert_eq!(run.stdout, path.as_bytes());
    }

    #[test]
    fn legacy_and_service_entry_points_preserve_raw_output_and_caps() {
        let args = ["-c".into(), "printf 'abcdef'; printf 'uvwxyz' >&2".into()];
        let limits = ProcessLimits {
            stdout_bytes: 3,
            stderr_bytes: 2,
            ..limits()
        };
        let direct =
            host_process::run(Path::new("/bin/sh"), &args, None, &limits, &|| false).unwrap();
        let legacy = crate::native_pipeline::process::run(
            Path::new("/bin/sh"),
            &args,
            None,
            &limits,
            &|| false,
        )
        .unwrap();
        for result in [&direct, &legacy] {
            assert!(result.succeeded());
            assert_eq!(result.stdout, b"abc");
            assert_eq!(result.stderr, b"uv");
            assert!(result.truncated);
            assert!(!result.cancelled);
            assert!(!result.timed_out);
            assert!(result.cleanup_error.is_none());
        }
        assert_eq!(direct.exit, legacy.exit);
    }
}
