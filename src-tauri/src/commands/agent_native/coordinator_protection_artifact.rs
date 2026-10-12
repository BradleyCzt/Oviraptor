// Verify untouched first-writer bytes via fixed no-follow directory handles.
fn native_protection_original_artifact(
    db: &rusqlite::Connection,
    target_dir: &Path,
    fact: &JsonValue,
    claim: &JsonValue,
) -> Result<JsonValue, String> {
    let writer = &fact["firstWriter"];
    let artifact = native_protection_text(fact, "artifactId")?;
    let slot = artifact
        .strip_suffix(".json")
        .filter(|s| s.len() >= 4 && s.bytes().all(|b| b.is_ascii_digit()))
        .ok_or("protection_original_artifact_name_invalid")?;
    let index = slot
        .parse::<u64>()
        .map_err(|_| "protection_original_artifact_name_invalid")?;
    if index == 0
        || format!("{index:04}.json") != artifact
        || writer["artifactId"] != artifact
        || Path::new(native_protection_text(writer, "directory")?)
            != target_dir.join(AGENT_HTTP_DIRECTORY)
        || Path::new(native_protection_text(claim, "targetDir")?) != target_dir
    {
        return Err("protection_original_directory_changed".into());
    }
    let actual_db: String = db
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name='main'",
            [],
            |r| r.get(0),
        )
        .map_err(|_| "protection_original_database_unavailable")?;
    let original_db = Path::new(native_protection_text(claim, "dbPath")?)
        .canonicalize()
        .map_err(|_| "protection_original_database_changed")?;
    if Path::new(&actual_db)
        .canonicalize()
        .map_err(|_| "protection_original_database_changed")?
        != original_db
    {
        return Err("protection_original_database_changed".into());
    }
    let directory = open_agent_artifact_directory(&target_dir.join(AGENT_HTTP_DIRECTORY))
        .ok_or("protection_original_directory_unavailable")?;
    native_protection_require_private_directory(&directory)?;
    let expected_metadata = writer["metadataBytes"]
        .as_u64()
        .filter(|n| *n > 0 && *n <= 1_048_576)
        .ok_or("protection_original_artifact_size_invalid")?;
    let expected_body = writer["bodyBytes"]
        .as_u64()
        .filter(|n| *n <= AGENT_MAX_RESPONSE_BYTES as u64)
        .ok_or("protection_original_artifact_size_invalid")?;
    let (bytes, metadata_len) =
        native_protection_read_private_file(&directory, artifact, 1_048_576)
            .ok_or("protection_original_metadata_unavailable")?;
    let (body, body_len) = native_protection_read_private_file(
        &directory,
        &format!("{slot}.body"),
        AGENT_MAX_RESPONSE_BYTES as u64,
    )
    .ok_or("protection_original_body_unavailable")?;
    use sha2::Digest;
    let metadata_hash = format!("{:x}", Sha256::digest(&bytes));
    let body_hash = format!("{:x}", Sha256::digest(&body));
    if metadata_len != expected_metadata
        || body_len != expected_body
        || writer["metadataSha256"] != metadata_hash
        || writer["bodySha256"] != body_hash
    {
        return Err("protection_original_artifact_bytes_changed".into());
    }
    let record: JsonValue =
        serde_json::from_slice(&bytes).map_err(|_| "protection_original_metadata_invalid")?;
    native_protection_exact_keys(
        &record,
        &[
            "index",
            "request",
            "response",
            "payloadFile",
            "payloadBytes",
            "requestPayloadFile",
        ],
    )?;
    let response = &record["response"];
    native_protection_exact_keys(
        response,
        &[
            "schemaVersion",
            "status",
            "contentType",
            "truncated",
            "bodySha256",
            "protectionCode",
            "provenance",
            "httpClaim",
            "preSendAuthorizationHash",
        ],
    )?;
    native_protection_exact_keys(
        &record["request"],
        &[
            "identity",
            "method",
            "url",
            "headerNames",
            "contractKey",
            "bodyBytes",
        ],
    )?;
    let bound = json!({"runId":claim["runId"],"invocationId":claim["invocationId"],"requestIndex":claim["requestIndex"],
        "budgetAttempt":claim["budgetAttempt"],"ordinal":claim["ordinal"],"requestHash":claim["requestHash"]});
    let content_type = fact["contentType"]
        .as_str()
        .ok_or("protection_original_metadata_invalid")?;
    let pre_hash =
        crate::agent_runtime::store::stable_hash(&fact["preSendAuthorization"].to_string());
    if record.to_string().as_bytes() != bytes
        || record["index"] != index
        || record["payloadFile"] != format!("{slot}.body")
        || record["payloadBytes"] != body_len
        || (!record["requestPayloadFile"].is_null()
            && record["requestPayloadFile"] != format!("{slot}.request"))
        || response["schemaVersion"] != 1
        || response["status"] != fact["status"]
        || response["contentType"] != content_type.chars().take(256).collect::<String>()
        || response["truncated"] != fact["truncated"]
        || response["bodySha256"] != body_hash
        || response["protectionCode"] != fact["protectionCode"]
        || response["provenance"] != "transport_observed_unsealed"
        || response["httpClaim"] != bound
        || response["preSendAuthorizationHash"] != pre_hash
    {
        return Err("protection_original_metadata_binding_changed".into());
    }
    // Repeat the frozen classifier on original bytes. A valid signature is
    // still not permission to turn a non-protection response into this source.
    let status = native_protection_positive(fact, "status")?;
    let text = String::from_utf8_lossy(&body);
    if status == 404
        || fact["classifierVersion"] != "native-directory-block-signal.v1"
        || !is_directory_block_signal(&format!(
            "http {status} {content_type} {}",
            text.chars().take(4_000).collect::<String>()
        ))
    {
        return Err("protection_original_classifier_changed".into());
    }
    native_protection_require_private_directory(&directory)?;
    Ok(
        json!({"artifactId":artifact,"metadataSha256":metadata_hash,"metadataBytes":metadata_len,
        "bodySha256":body_hash,"bodyBytes":body_len,"preSendAuthorizationHash":pre_hash}),
    )
}

#[cfg(unix)]
fn native_protection_require_private_directory(directory: &std::fs::File) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let metadata = directory
        .metadata()
        .map_err(|_| "protection_original_directory_unavailable")?;
    if !metadata.is_dir() || metadata.mode() & 0o7777 != 0o700 {
        return Err("protection_original_directory_not_private".into());
    }
    Ok(())
}
#[cfg(not(unix))]
fn native_protection_require_private_directory(_: &std::fs::File) -> Result<(), String> {
    Err("protection_original_private_reader_unavailable".into())
}

#[cfg(unix)]
fn native_protection_read_private_file(
    directory: &std::fs::File,
    name: &str,
    maximum: u64,
) -> Option<(Vec<u8>, u64)> {
    use std::{
        io::Read,
        os::fd::{AsRawFd, FromRawFd},
        os::unix::fs::MetadataExt,
    };
    if name.is_empty()
        || name == "."
        || name == ".."
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
    {
        return None;
    }
    let name = std::ffi::CString::new(name).ok()?;
    // SAFETY: this single component is opened relative to a verified live
    // directory fd; only a successful openat fd is transferred to File.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return None;
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let before = file.metadata().ok()?;
    if !before.is_file()
        || before.nlink() != 1
        || before.mode() & 0o7777 != 0o600
        || before.len() > maximum
    {
        return None;
    }
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(maximum.checked_add(1)?)
        .read_to_end(&mut bytes)
        .ok()?;
    let after = file.metadata().ok()?;
    if !after.is_file()
        || after.nlink() != 1
        || after.mode() & 0o7777 != 0o600
        || after.len() != before.len()
        || bytes.len() as u64 != after.len()
        || before.dev() != after.dev()
        || before.ino() != after.ino()
    {
        return None;
    }
    Some((bytes, after.len()))
}
#[cfg(not(unix))]
fn native_protection_read_private_file(
    _: &std::fs::File,
    _: &str,
    _: u64,
) -> Option<(Vec<u8>, u64)> {
    None
}
