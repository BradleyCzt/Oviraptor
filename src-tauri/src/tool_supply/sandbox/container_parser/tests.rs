use super::*;
fn request(runtime: &Runtime, code: &str) -> Request {
    Request {
        code: code.into(),
        input: json!({"Cookie":"session=fixture-account-a","Token":"fixture-account-b","password":"fixture-password"}),
        limits: Limits {
            memory_mib: 32,
            timeout_seconds: 3,
            output_bytes: 4096,
        },
        runtime_digest: runtime.digest().into(),
    }
}

#[test]
fn container_parser_real_execution_retains_raw_input_and_proves_cgroup_cleanup() {
    let runtime = Runtime::inspect().unwrap();
    let request = request(&runtime, "def analyze(data):\n return data");
    let got = runtime.execute(&request, &|| false).unwrap();
    assert_eq!(got.exit_code, 0, "{}", got.stderr);
    assert!(got.cleanup_verified);
    assert!(!got.oom_killed);
    let output: Value = serde_json::from_str(&got.stdout).unwrap();
    assert_eq!(output["result"], request.input);
    assert_eq!(output["codeSha256"], got.code_sha256);
    assert_eq!(output["inputSha256"], got.input_sha256);
    assert!(output["resourceUsage"]["maxRssKib"].as_u64().unwrap() > 0);
}

#[test]
fn container_parser_real_host_files_network_fork_shell_and_root_writes_are_denied() {
    let runtime = Runtime::inspect().unwrap();
    let request = request(
        &runtime,
        r#"def analyze(data):
 import os,socket
 result={}
 for key,operation in [('host',lambda:open('/Users/swyiic/.ssh/id_rsa').read()),('socket',lambda:socket.socket()),('fork',lambda:os.fork()),('shell',lambda:os.execl('/bin/sh','sh','-c','echo unsafe')),('root_write',lambda:open('/etc/escape','w'))]:
  try: operation(); result[key]='ALLOWED'
  except OSError: result[key]='denied'
 return result
"#,
    );
    let got = runtime.execute(&request, &|| false).unwrap();
    assert_eq!(got.exit_code, 0, "{}", got.stderr);
    let output: Value = serde_json::from_str(&got.stdout).unwrap();
    assert_eq!(
        output["result"],
        json!({"host":"denied","socket":"denied","fork":"denied","shell":"denied","root_write":"denied"})
    );
    assert!(got.cleanup_verified);
}

#[test]
fn container_parser_real_oom_timeout_output_error_and_cancel_keep_exit_receipts() {
    let runtime = Runtime::inspect().unwrap();
    for (code, reason) in [
        (
            "def analyze(data):\n x=[]\n while True: x.append(bytearray(1048576))",
            "oom",
        ),
        ("def analyze(data):\n while True: pass", "timeout"),
        (
            "def analyze(data):\n while True: print('x'*4096,flush=True)",
            "output_limit_exceeded",
        ),
        (
            "def analyze(data):\n raise ValueError('original-error')",
            "error",
        ),
    ] {
        let got = runtime
            .execute(&request(&runtime, code), &|| false)
            .unwrap();
        assert!(got.cleanup_verified);
        match reason {
            "oom" => {
                assert!(got.oom_killed);
                assert_eq!(got.exit_code, 137);
            }
            "error" => {
                assert_ne!(got.exit_code, 0);
                assert!(got.stderr.contains("original-error"));
            }
            _ => assert_eq!(got.stop_reason, reason),
        }
        assert!(got.stdout.len() + got.stderr.len() <= 4096);
    }
    let started = Instant::now();
    let got = runtime
        .execute(
            &request(&runtime, "def analyze(data):\n while True: pass"),
            &|| started.elapsed() > Duration::from_secs(1),
        )
        .unwrap();
    assert_eq!(got.stop_reason, "cancelled");
    assert!(got.cleanup_verified);
}

#[test]
fn container_parser_refuses_unapproved_runtime_and_oversized_input_before_create() {
    let runtime = Runtime::inspect().unwrap();
    let mut req = request(&runtime, "def analyze(data): return data");
    req.runtime_digest = "sha256:caller".into();
    assert_eq!(
        runtime.execute(&req, &|| false).unwrap_err(),
        "container_parser_runtime_changed"
    );
    req.runtime_digest = runtime.digest().into();
    req.code = "x".repeat(MAX_CODE + 1);
    assert_eq!(
        runtime.execute(&req, &|| false).unwrap_err(),
        "container_parser_input_size_invalid"
    );
    assert_eq!(
        runtime
            .execute(
                &request(&runtime, "def analyze(data): return data"),
                &|| true
            )
            .unwrap_err(),
        "container_parser_cancelled_before_create"
    );
}

#[test]
fn container_parser_real_parallel_inputs_outputs_and_environment_are_isolated() {
    let runtime = Runtime::inspect().unwrap();
    let mut left=request(&runtime,"def analyze(data):\n import os,time\n open('/output/own','w').write(data['Cookie'])\n time.sleep(0.5)\n return {'input':data,'stored':open('/output/own').read(),'environment':dict(os.environ)}");
    let mut right=request(&runtime,"def analyze(data):\n import os,time\n time.sleep(0.2)\n return {'input':data,'otherFilePresent':os.path.exists('/output/own'),'environment':dict(os.environ)}");
    left.limits.timeout_seconds = 5;
    right.limits.timeout_seconds = 5;
    right.input = json!({"Cookie":"session=fixture-other-run"});
    let (a, b) = std::thread::scope(|scope| {
        let a = scope.spawn(|| runtime.execute(&left, &|| false).unwrap());
        let b = scope.spawn(|| runtime.execute(&right, &|| false).unwrap());
        (a.join().unwrap(), b.join().unwrap())
    });
    assert_ne!(a.container_id, b.container_id);
    assert!(a.cleanup_verified && b.cleanup_verified);
    assert_eq!(a.exit_code, 0, "{}", a.stderr);
    assert_eq!(b.exit_code, 0, "{}", b.stderr);
    let a: Value = serde_json::from_str(&a.stdout).unwrap();
    let b: Value = serde_json::from_str(&b.stdout).unwrap();
    assert_eq!(a["result"]["stored"], left.input["Cookie"]);
    assert_eq!(b["result"]["input"], right.input);
    assert_eq!(b["result"]["otherFilePresent"], false);
    assert_eq!(a["result"]["environment"], json!({}));
    assert_eq!(b["result"]["environment"], json!({}));
}

#[test]
fn container_parser_guardian_protocol_is_bounded_and_rejects_extra_authority() {
    let mut malformed = std::io::Cursor::new(vec![b'x'; 512 * 1024 + 1]);
    assert_eq!(
        read_guardian_request(&mut malformed).unwrap_err(),
        "container_parser_guardian_request_too_large"
    );
    let mut incomplete = std::io::Cursor::new(b"{}");
    assert_eq!(
        read_guardian_request(&mut incomplete).unwrap_err(),
        "container_parser_guardian_request_incomplete"
    );
    let runtime = Runtime::inspect().unwrap();
    let mut value =
        serde_json::to_value(request(&runtime, "def analyze(data): return data")).unwrap();
    value["hostMount"] = json!("/Users/swyiic");
    let mut encoded = value.to_string().into_bytes();
    encoded.push(b'\n');
    assert_eq!(
        read_guardian_request(&mut std::io::Cursor::new(encoded)).unwrap_err(),
        "container_parser_guardian_request_invalid"
    );
}
