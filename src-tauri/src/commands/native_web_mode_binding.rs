// Ordinary Web only. Compact mode proof reuses the original private key; the
// mutable live-status full startup snapshot is NOT recomputed during Root read.
use crate::agent_runtime::web_mode::{self, ModeFact, VerifiedMode, WebMode};

fn private_web_mode_on(
    db: &rusqlite::Connection,
    scan: &str,
    attempt: i64,
) -> Result<VerifiedMode, String> {
    let (fact, signature) = web_mode::stored_fact(db, scan, attempt)?;
    let work: String = db
        .query_row(
            "SELECT work_dir FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=?2",
            params![scan, attempt],
            |r| r.get(0),
        )
        .map_err(|_| "web_mode_attempt_missing")?;
    if fact.work_dir != work {
        return Err("web_mode_directory_changed".into());
    }
    let directory = WebBindingDirectory::open(Path::new(&work))?;
    let marker = directory
        .read(".oviraptor-scan-id", 1024, false, false)?
        .ok_or("web_mode_marker_missing")?;
    let key = directory
        .read(WEB_DISPATCH_KEY_FILE, 32, false, false)?
        .ok_or("web_mode_key_missing")?;
    let task = directory
        .read("task.json", WEB_DISPATCH_BINDING_LIMIT, false, false)?
        .ok_or("web_mode_task_missing")?;
    let document: JsonValue = serde_json::from_slice(&task).map_err(|_| "web_mode_task_invalid")?;
    let bindings: i64 = db.query_row("SELECT
        (SELECT count(*) FROM native_web_dispatch_bindings WHERE scan_id=?1 AND attempt_number=?2 AND branch='web')+
        (SELECT count(*) FROM agent_financial_startup_bindings WHERE scan_id=?1 AND attempt_number=?2)",
        params![scan,attempt],|r|r.get(0)).map_err(|_|"web_mode_startup_proof_missing")?;
    if bindings == 0 {
        return Err("web_mode_startup_proof_missing".into());
    }
    if bindings != 1 {
        return Err("web_mode_startup_proof_conflict".into());
    }
    let (schema, tag, financial, financial_approval): (i64, Vec<u8>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT schema_version,binding_tag,NULL,NULL FROM native_web_dispatch_bindings
        WHERE scan_id=?1 AND attempt_number=?2 AND branch='web'
        UNION ALL SELECT 1,binding_tag,binding_fact_json,approval_id FROM agent_financial_startup_bindings
        WHERE scan_id=?1 AND attempt_number=?2",
            params![scan, attempt],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(|_| "web_mode_startup_proof_missing")?;
    if let Some(raw) = financial {
        let binding: JsonValue =
            serde_json::from_str(&raw).map_err(|_| "financial_continuation_startup_invalid")?;
        let canonical = binding.to_string();
        if raw != canonical
            || binding["domain"] != "oviraptor.financial-continuation.v1"
            || binding.as_object().map(|o| o.len()) != Some(6)
            || binding["approvalId"].as_str() != financial_approval.as_deref()
            || crate::agent_runtime::store::stable_hash(&raw) != fact.runtime_hash
            || document["financialContinuation"]["approvalId"] != binding["approvalId"]
            || document["financialContinuation"]["proposalHash"] != binding["proposalHash"]
            || document["taskPurpose"] != "new_work_after_original_cost_confirmation"
        {
            return Err("financial_continuation_startup_changed".into());
        }
        aws_lc_rs::hmac::verify(
            &aws_lc_rs::hmac::Key::new(aws_lc_rs::hmac::HMAC_SHA256, &key),
            raw.as_bytes(),
            &tag,
        )
        .map_err(|_| "financial_continuation_startup_signature_invalid")?;
    }
    let tags = tag.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let targets = mode_task_targets(&document)?;
    let policy = &document["effectiveWebPolicy"];
    let current: String = db
        .query_row(
            "SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id=?1",
            [scan],
            |r| r.get(0),
        )
        .map_err(|_| "web_mode_current_policy_missing")?;
    let current: JsonValue =
        serde_json::from_str(&current).map_err(|_| "web_mode_current_policy_invalid")?;
    if schema != 1
        || tag.len() != 32
        || marker != scan.as_bytes()
        || fact.startup_tag != tags
        || fact.task_hash != format!("{:x}", Sha256::digest(&task))
        || document["scanId"] != scan
        || fact.targets != targets
        || fact.policy_hash != crate::agent_runtime::store::stable_hash(&policy.to_string())
        || WebMode::from_policy(policy)? != fact.mode
        || WebMode::from_policy(&current)? != fact.mode
        || web_mode::action::read_hash(db, scan)? != fact.creation_action_hash
        || web_mode::draft::read(db, scan)? != (fact.draft_id.clone(), fact.mode)
    {
        return Err("web_mode_private_inputs_changed".into());
    }
    directory.check_identity()?;
    VerifiedMode::verify(fact, &key, &signature)
}

fn mode_task_targets(task: &JsonValue) -> Result<Vec<String>, String> {
    let rows = task["targets"]
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= 200)
        .ok_or("web_mode_targets_invalid")?;
    let mut targets = rows
        .iter()
        .map(|row| {
            row["url"]
                .as_str()
                .map(str::to_string)
                .ok_or("web_mode_targets_invalid")
        })
        .collect::<Result<Vec<_>, _>>()?;
    targets.sort();
    if targets.windows(2).any(|s| s[0] == s[1]) {
        return Err("web_mode_targets_invalid".into());
    }
    Ok(targets)
}

fn require_web_start_mode_on(
    db: &rusqlite::Connection,
    scan: &str,
    previous: i64,
    continuation: bool,
) -> Result<WebMode, String> {
    web_mode::action::read_hash(db, scan)?;
    let (_, mode) = web_mode::draft::read(db, scan)?;
    let policy: String = db
        .query_row(
            "SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id=?1",
            [scan],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let policy: JsonValue = serde_json::from_str(&policy).map_err(|_| "web_mode_policy_invalid")?;
    if WebMode::from_policy(&policy)? != mode {
        return Err("web_mode_draft_policy_changed".into());
    }
    if continuation && private_web_mode_on(db, scan, previous)?.fact().mode != mode {
        return Err("web_mode_parent_conflict".into());
    }
    Ok(mode)
}

fn register_private_web_mode_on(
    db: &rusqlite::Connection,
    startup: &WebStartup,
    runtime: &JsonValue,
) -> Result<(), String> {
    let directory = WebBindingDirectory::open(&startup.files.path)?;
    let attempt = i64::from(startup.attempt);
    let (draft_id, mode) = web_mode::draft::read(db, &startup.scan_id)?;
    let task = directory
        .read("task.json", WEB_DISPATCH_BINDING_LIMIT, false, false)?
        .ok_or("web_mode_task_missing")?;
    let document: JsonValue = serde_json::from_slice(&task).map_err(|_| "web_mode_task_invalid")?;
    if document["scanId"] != startup.scan_id
        || WebMode::from_policy(&document["effectiveWebPolicy"])? != mode
    {
        return Err("web_mode_task_scope_changed".into());
    }
    let parent = if startup.execution_mode == "resume" {
        let original = private_web_mode_on(db, &startup.scan_id, startup.previous_attempt)?;
        if original.fact().mode != mode {
            return Err("web_mode_parent_conflict".into());
        }
        Some(original.fact().receipt_id.clone())
    } else {
        None
    };
    let tag: Vec<u8> = db
        .query_row(
            "SELECT binding_tag FROM native_web_dispatch_bindings WHERE scan_id=?1
        AND attempt_number=?2 AND branch='web' AND schema_version=1",
            params![startup.scan_id, attempt],
            |r| r.get(0),
        )
        .map_err(|_| "web_mode_startup_proof_missing")?;
    let fact = ModeFact {
        schema_version: 1,
        receipt_id: Uuid::new_v4().to_string(),
        draft_id,
        scan_id: startup.scan_id.clone(),
        attempt_number: attempt,
        execution_mode: startup.execution_mode.clone(),
        parent_receipt_id: parent,
        creation_action_hash: web_mode::action::read_hash(db, &startup.scan_id)?,
        mode,
        work_dir: startup
            .files
            .path
            .to_str()
            .ok_or("web_mode_path_invalid")?
            .into(),
        task_hash: format!("{:x}", Sha256::digest(&task)),
        policy_hash: crate::agent_runtime::store::stable_hash(
            &document["effectiveWebPolicy"].to_string(),
        ),
        runtime_hash: crate::agent_runtime::store::stable_hash(&runtime.to_string()),
        startup_tag: tag.iter().map(|b| format!("{b:02x}")).collect(),
        targets: mode_task_targets(&document)?,
    };
    let key = directory
        .read(WEB_DISPATCH_KEY_FILE, 32, false, false)?
        .ok_or("web_mode_key_missing")?;
    if key.len() != 32 {
        return Err("web_mode_key_invalid".into());
    }
    let signature = aws_lc_rs::hmac::sign(
        &aws_lc_rs::hmac::Key::new(aws_lc_rs::hmac::HMAC_SHA256, &key),
        &fact.message()?,
    );
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    // One setter, not nested inside either Root writer. Only this receipt may
    // be written; late triggers cannot mutate the already signed startup inputs.
    db.authorizer(Some(|c: AuthContext<'_>| match c.action {
        AuthAction::Insert {
            table_name: "native_web_mode_receipts",
        } if c.database_name == Some("main") && c.accessor.is_none() => Authorization::Allow,
        AuthAction::Read { .. }
        | AuthAction::Select
        | AuthAction::Function { .. }
        | AuthAction::Transaction { .. }
        | AuthAction::Savepoint { .. }
        | AuthAction::Recursive => Authorization::Allow,
        _ => Authorization::Deny,
    }))
    .map_err(|e| e.to_string())?;
    let result = (|| {
        let text = serde_json::to_string(&fact).map_err(|e| e.to_string())?;
        let n=db.execute("INSERT INTO native_web_mode_receipts(scan_id,attempt_number,receipt_id,fact_json,signature) VALUES(?1,?2,?3,?4,?5)",
            params![fact.scan_id,fact.attempt_number,fact.receipt_id,text,signature.as_ref()]).map_err(|e|e.to_string())?;
        if n != 1 || private_web_mode_on(db, &startup.scan_id, attempt)?.fact() != &fact {
            return Err("web_mode_receipt_unconfirmed".into());
        }
        // Full original HMAC still matches after the FINAL publication write.
        verify_web_dispatch_binding_in(db, &startup.scan_id, attempt, &directory, runtime)
    })();
    let clear = db
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .map_err(|e| e.to_string());
    result?;
    clear
}
