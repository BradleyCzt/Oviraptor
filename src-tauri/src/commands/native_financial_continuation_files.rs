// Exact historical frontend bytes copied into an exclusively created attempt.
// Compensation touches only these new files; no historical artifact is deleted.
struct FinancialContinuationFiles {
    attempt: WebStartFiles,
    target: PathBuf,
    source_target: PathBuf,
    evidence_hash: String,
    evidence: Vec<u8>,
    nested: Vec<(PathBuf, File)>,
}

fn financial_read_evidence(path: &Path) -> Result<Vec<u8>, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let mut file = options
        .open(path)
        .map_err(|_| "financial_continuation_evidence_unavailable")?;
    let metadata = file
        .metadata()
        .map_err(|_| "financial_continuation_evidence_unavailable")?;
    if !metadata.is_file() || metadata.len() > 256 * 1024 {
        return Err("financial_continuation_evidence_invalid".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err("financial_continuation_evidence_invalid".into());
        }
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(256 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 256 * 1024 {
        return Err("financial_continuation_evidence_invalid".into());
    }
    Ok(bytes)
}

impl FinancialContinuationFiles {
    fn allocate_in(
        db: &rusqlite::Connection,
        app_data: &Path,
        authorization: &FinancialContinuationAuthorization,
    ) -> Result<Self, String> {
        let p = &authorization.proposal;
        let scan = p["scanId"]
            .as_str()
            .ok_or("financial_continuation_scope_invalid")?;
        let source = p["rootRunId"]
            .as_str()
            .ok_or("financial_continuation_scope_invalid")?;
        let source_target = resolve_agent_evidence_location(db, source)?;
        let evidence = financial_read_evidence(&source_target.join("frontend-evidence.json"))?;
        let text = std::str::from_utf8(&evidence)
            .map_err(|_| "financial_continuation_evidence_invalid")?;
        let evidence_hash = crate::agent_runtime::store::stable_hash(text);
        let document: JsonValue = serde_json::from_slice(&evidence)
            .map_err(|_| "financial_continuation_evidence_invalid")?;
        if !document.is_object()
            || document.get("error").is_some()
            || document["url"] != p["targetKey"]
        {
            return Err("financial_continuation_evidence_invalid".into());
        }
        // The original provider call committed these exact bytes. Confirmation
        // of its fee does not authorize substituting later frontend evidence.
        let raw: String = db
            .query_row(
                "SELECT fact_json FROM agent_root_tick_receipts
            WHERE root_run_id=?1 AND round=1 AND phase='request'",
                [source],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let request: JsonValue = serde_json::from_str(&raw)
            .map_err(|_| "financial_continuation_original_request_invalid")?;
        if request["request"]["basis"]["evidenceFileHash"] != evidence_hash {
            return Err("financial_continuation_original_evidence_changed".into());
        }
        let root = web_start_root(app_data, scan)?;
        let attempt = u32::try_from(authorization.plan.attempt_number)
            .map_err(|_| "financial_continuation_attempt_invalid")?;
        if web_start_attempt_number(db, scan, &root, authorization.plan.attempt_number - 1)?
            != attempt
        {
            return Err("financial_continuation_attempt_directory_changed".into());
        }
        let files = WebStartFiles::allocate(&root, scan, attempt)?;
        let target = files.path.join("url-pipeline/target-00001");
        let mut result = Self {
            attempt: files,
            target,
            source_target,
            evidence_hash,
            evidence,
            nested: Vec::new(),
        };
        for directory in [
            result.attempt.path.join("url-pipeline"),
            result.target.clone(),
        ] {
            fs::create_dir(&directory).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                    .map_err(|e| e.to_string())?;
            }
            result.nested.push((
                directory.clone(),
                File::open(&directory).map_err(|e| e.to_string())?,
            ));
        }
        let target_directory = WebBindingDirectory::open(&result.target)?;
        let mut marker = target_directory
            .open_file(".oviraptor-scan-id", true)
            .map_err(|e| e.to_string())?;
        marker
            .write_all(scan.as_bytes())
            .and_then(|()| marker.sync_all())
            .map_err(|e| e.to_string())?;
        let mut file = target_directory
            .open_file("frontend-evidence.json", true)
            .map_err(|e| e.to_string())?;
        file.write_all(&result.evidence)
            .and_then(|()| file.sync_all())
            .map_err(|e| e.to_string())?;
        target_directory.sync()?;
        let original_mode = private_web_mode_on(db, scan, authorization.plan.attempt_number - 1)?;
        let source_directory =
            WebBindingDirectory::open(Path::new(&original_mode.fact().work_dir))?;
        let task = source_directory
            .read("task.json", WEB_DISPATCH_BINDING_LIMIT, false, false)?
            .ok_or("web_mode_task_missing")?;
        let mut task: JsonValue =
            serde_json::from_slice(&task).map_err(|_| "web_mode_task_invalid")?;
        let project: i64 = db
            .query_row(
                "SELECT project_id FROM sentinel_scans WHERE id=?1",
                [scan],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if task["projectId"] != project {
            return Err("financial_continuation_original_project_changed".into());
        }
        task["attemptNumber"] = authorization.plan.attempt_number.into();
        task["executionMode"] = "resume".into();
        task["taskPurpose"] = "new_work_after_original_cost_confirmation".into();
        task["financialContinuation"] = json!({"approvalId":authorization.approval_id,
            "proposalHash":p["proposalHash"],"taskDescription":p["taskDescription"],"historicalEvidence":true});
        task["targets"] = json!([{"url":p["targetKey"]}]);
        let directory = WebBindingDirectory::open(&result.attempt.path)?;
        let mut file = directory
            .open_file("task.json", true)
            .map_err(|e| e.to_string())?;
        file.write_all(task.to_string().as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|e| e.to_string())?;
        source_directory.check_identity()?;
        result.verify()?;
        Ok(result)
    }

    fn verify(&self) -> Result<(), String> {
        if financial_read_evidence(&self.source_target.join("frontend-evidence.json"))?
            != self.evidence
            || financial_read_evidence(&self.target.join("frontend-evidence.json"))?
                != self.evidence
        {
            return Err("financial_continuation_evidence_changed".into());
        }
        checked_evidence_directory(&self.attempt.path, &self.target, &self.attempt.scan_id)?;
        WebBindingDirectory::open(&self.attempt.path)?.sync()
    }

    fn publish_mode_in(
        &self,
        db: &rusqlite::Connection,
        authorization: &FinancialContinuationAuthorization,
    ) -> Result<crate::agent_runtime::web_mode::VerifiedMode, String> {
        use crate::agent_runtime::web_mode::{ModeFact, VerifiedMode};
        let scan = &self.attempt.scan_id;
        let old = private_web_mode_on(db, scan, authorization.plan.attempt_number - 1)?;
        let directory = WebBindingDirectory::open(&self.attempt.path)?;
        let task = directory
            .read("task.json", WEB_DISPATCH_BINDING_LIMIT, false, true)?
            .ok_or("web_mode_task_missing")?;
        let document: JsonValue =
            serde_json::from_slice(&task).map_err(|_| "web_mode_task_invalid")?;
        let mut key = [0u8; 32];
        aws_lc_rs::rand::fill(&mut key).map_err(|_| "web_binding_entropy_unavailable")?;
        let mut file = directory
            .open_file(WEB_DISPATCH_KEY_FILE, true)
            .map_err(|e| e.to_string())?;
        file.write_all(&key)
            .and_then(|()| file.sync_all())
            .map_err(|e| e.to_string())?;
        let runtime = json!({"domain":"oviraptor.financial-continuation.v1","approvalId":authorization.approval_id,
            "proposalHash":authorization.proposal["proposalHash"],"frontendEvidenceHash":self.evidence_hash,
            "newRootRunId":authorization.new_root,"newPlanHash":authorization.plan.hash()});
        let tag = aws_lc_rs::hmac::sign(
            &aws_lc_rs::hmac::Key::new(aws_lc_rs::hmac::HMAC_SHA256, &key),
            runtime.to_string().as_bytes(),
        );
        let fact = ModeFact {
            schema_version: 1,
            receipt_id: Uuid::new_v4().to_string(),
            draft_id: old.fact().draft_id.clone(),
            scan_id: scan.clone(),
            attempt_number: authorization.plan.attempt_number,
            execution_mode: "resume".into(),
            parent_receipt_id: Some(old.fact().receipt_id.clone()),
            creation_action_hash: old.fact().creation_action_hash.clone(),
            mode: old.fact().mode,
            work_dir: self
                .attempt
                .path
                .to_str()
                .ok_or("web_mode_path_invalid")?
                .into(),
            task_hash: format!("{:x}", Sha256::digest(&task)),
            policy_hash: crate::agent_runtime::store::stable_hash(
                &document["effectiveWebPolicy"].to_string(),
            ),
            runtime_hash: crate::agent_runtime::store::stable_hash(&runtime.to_string()),
            startup_tag: tag.as_ref().iter().map(|b| format!("{b:02x}")).collect(),
            targets: mode_task_targets(&document)?,
        };
        let signature = aws_lc_rs::hmac::sign(
            &aws_lc_rs::hmac::Key::new(aws_lc_rs::hmac::HMAC_SHA256, &key),
            &fact.message()?,
        );
        db.execute("INSERT INTO agent_financial_startup_bindings(scan_id,attempt_number,approval_id,binding_tag,binding_fact_json)
            VALUES(?1,?2,?3,?4,?5)",params![scan,authorization.plan.attempt_number,authorization.approval_id,tag.as_ref(),runtime.to_string()]).map_err(|e|e.to_string())?;
        db.execute("INSERT INTO native_web_mode_receipts(scan_id,attempt_number,receipt_id,fact_json,signature)
            VALUES(?1,?2,?3,?4,?5)",params![scan,authorization.plan.attempt_number,fact.receipt_id,serde_json::to_string(&fact).map_err(|e|e.to_string())?,signature.as_ref()])
            .map_err(|e|e.to_string())?;
        directory.sync()?;
        let verified = VerifiedMode::verify(fact.clone(), &key, signature.as_ref())?;
        if private_web_mode_on(db, scan, authorization.plan.attempt_number)?.fact() != &fact {
            return Err("financial_continuation_new_mode_changed".into());
        }
        Ok(verified)
    }
}

impl Drop for FinancialContinuationFiles {
    fn drop(&mut self) {
        if self.attempt.preserve {
            return;
        }
        for (path, handle) in self.nested.iter().rev() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                let Ok(original) = handle.metadata() else {
                    continue;
                };
                if !fs::symlink_metadata(path)
                    .is_ok_and(|m| (m.dev(), m.ino()) == (original.dev(), original.ino()))
                {
                    continue;
                }
            }
            if path == &self.target {
                for name in ["frontend-evidence.json", ".oviraptor-scan-id"] {
                    let file = path.join(name);
                    if fs::symlink_metadata(&file)
                        .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
                    {
                        let _ = fs::remove_file(file);
                    }
                }
            }
            let _ = fs::remove_dir(path);
        }
    }
}
