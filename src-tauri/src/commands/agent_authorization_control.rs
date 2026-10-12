/// Explicit operator input for one cross-object authorization control group.
/// This command is deliberately not an Agent tool: model text, imported JSON,
/// and `ownershipHints` cannot create an ownership assertion.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveAuthorizationControlInput {
    scan_id: String,
    attempt_number: i64,
    target_url: String,
    contract_key: String,
    owner_object_url: String,
    tester_control_url: String,
    object_query_key: String,
    owner_object_value: String,
    tester_object_value: String,
    response_object_pointer: String,
    owner_identity: String,
    tester_identity: String,
}

/// Read-only form metadata. Never return cookies, headers or captured session JSON.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationControlSetup {
    attempt_number: i64,
    identity_ids: Vec<String>,
    contract_keys: Vec<String>,
}

/// Private, raw response facts for one *actual* brokered request. Never send
/// `body` to the model or the collaboration timeline: it can contain secrets.
#[derive(Clone, Debug)]
struct AuthorizationProbeResponse {
    attempt_number: i64,
    contract_key: String,
    identity: String,
    url: String,
    status: u16,
    content_type: String,
    redirect_code: String,
    cache_state: String,
    truncated: bool,
    body: String,
}

/// The three sides are ordered A→X, B→X, B→Y. A/B differences on one URL
/// only prove personalization; they cannot establish object ownership.
fn verify_authorization_control_group(
    control: &SaveAuthorizationControlInput,
    sides: [&AuthorizationProbeResponse; 3],
) -> Result<(), &'static str> {
    let expected = [
        (&control.owner_identity, &control.owner_object_url, &control.owner_object_value),
        (&control.tester_identity, &control.owner_object_url, &control.owner_object_value),
        (&control.tester_identity, &control.tester_control_url, &control.tester_object_value),
    ];
    for (side, (identity, url, object)) in sides.iter().zip(expected) {
        if side.attempt_number != control.attempt_number
            || side.contract_key != control.contract_key
            || side.identity != *identity
            || side.url != *url
        {
            return Err("authorization_side_binding_invalid");
        }
        if side.status != 200 {
            return Err("authorization_side_not_successful");
        }
        let mime = side.content_type.to_ascii_lowercase();
        if side.truncated || !side.redirect_code.is_empty() || side.cache_state != "miss"
            || !mime.contains("json") || mime.contains("problem+json")
            || is_directory_block_signal(&side.body)
        {
            return Err("authorization_side_unusable");
        }
        let body: serde_json::Value = serde_json::from_str(&side.body)
            .map_err(|_| "authorization_side_unusable")?;
        if let Some(object) = body.as_object() {
            if ["error", "errors", "exception", "challenge", "captcha"].iter()
                .any(|key| object.contains_key(*key))
            {
                return Err("authorization_side_unusable");
            }
        }
        let Some(actual) = body.pointer(&control.response_object_pointer) else {
            return Err("authorization_object_pointer_missing");
        };
        let actual = match actual {
            serde_json::Value::String(text) => text.clone(),
            serde_json::Value::Number(number) => number.to_string(),
            _ => return Err("authorization_object_pointer_invalid"),
        };
        if actual != *object {
            return Err("authorization_cross_object_not_observed");
        }
    }
    // The unauthorized account must see the same *object* that its owner sees,
    // not just a JSON error envelope that happens to echo the requested id.
    let parent = control.response_object_pointer.rsplit_once('/')
        .map(|(path, _)| path).unwrap_or("");
    let owner: serde_json::Value = serde_json::from_str(&sides[0].body)
        .map_err(|_| "authorization_side_unusable")?;
    let cross: serde_json::Value = serde_json::from_str(&sides[1].body)
        .map_err(|_| "authorization_side_unusable")?;
    // An endpoint that merely reflects `?id=X` is not evidence that the
    // object was fetched. Require actual object data besides its identifier.
    let Some(owner_fields) = owner.pointer(parent).and_then(serde_json::Value::as_object) else {
        return Err("authorization_cross_object_not_comparable");
    };
    let id_field = control.response_object_pointer.rsplit('/').next().unwrap_or_default()
        .replace("~1", "/").replace("~0", "~");
    if !owner_fields.iter().any(|(key, value)| {
        key != &id_field && !value.is_null() && value != "" && value != &serde_json::json!({})
            && value != &serde_json::json!([])
    }) {
        return Err("authorization_cross_object_not_comparable");
    }
    if owner.pointer(parent) != cross.pointer(parent) {
        return Err("authorization_cross_object_not_comparable");
    }
    Ok(())
}

fn load_authorization_controls(
    connection: &rusqlite::Connection, context: &AgentRunContext,
) -> Result<Vec<SaveAuthorizationControlInput>, String> {
    let mut statement = connection.prepare(
        "SELECT contract_key,owner_object_url,tester_control_url,object_query_key,owner_object_value,\
         tester_object_value,response_object_pointer,owner_identity,tester_identity \
         FROM agent_authorization_controls WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 ORDER BY contract_key",
    ).map_err(|_| "authorization_control_lookup_failed".to_string())?;
    let rows = statement.query_map(params![context.scan_id, context.attempt_number, context.target_url], |row| {
        Ok(SaveAuthorizationControlInput {
            scan_id: context.scan_id.clone(), attempt_number: context.attempt_number,
            target_url: context.target_url.clone(), contract_key: row.get(0)?,
            owner_object_url: row.get(1)?, tester_control_url: row.get(2)?,
            object_query_key: row.get(3)?, owner_object_value: row.get(4)?,
            tester_object_value: row.get(5)?, response_object_pointer: row.get(6)?,
            owner_identity: row.get(7)?, tester_identity: row.get(8)?,
        })
    }).map_err(|_| "authorization_control_lookup_failed".to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|_| "authorization_control_lookup_failed".to_string())?;
    Ok(rows)
}

fn authorization_control_setup_in(
    connection: &rusqlite::Connection,
    app_data_dir: &Path,
    scan_id: &str,
    target_url: &str,
) -> Result<AuthorizationControlSetup, String> {
    let (count, status, scan_type): (i64, String, String) = connection.query_row(
        "SELECT attempt_count,status,scan_type FROM sentinel_scans WHERE id=?1",
        [scan_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).map_err(|_| "authorization_control_task_binding_invalid".to_string())?;
    if status != "draft" || scan_type != "web" || scan_id.trim().is_empty()
        || matches!(scan_id, "." | "..") || scan_id.contains(['/', '\\'])
    {
        return Err("authorization_control_task_binding_invalid".into());
    }
    let target_exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_targets WHERE scan_id=?1 AND url=?2)",
        params![scan_id, target_url],
        |row| row.get(0),
    ).map_err(|_| "authorization_target_lookup_failed".to_string())?;
    if !target_exists {
        return Err("authorization_control_task_binding_invalid".into());
    }
    let (mode, identity_ids) = crate::auth_session::validated_scan_identities(
        connection, scan_id, target_url,
    )?;
    if mode != crate::auth_session::ScanIdentityMode::IdentitySet || identity_ids.len() < 2 {
        return Err("authorization_control_requires_two_identities".into());
    }
    let attempt_number = next_scan_attempt_number(
        &neutral_scan_work_root(app_data_dir).join(scan_id),
        count.saturating_add(1).clamp(1, i64::from(u32::MAX)) as u32,
    ) as i64;
    let mut statement = connection.prepare(
        "SELECT contract_key FROM agent_authorization_controls WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 ORDER BY contract_key",
    ).map_err(|_| "authorization_control_lookup_failed".to_string())?;
    let contract_keys = statement.query_map(params![scan_id, attempt_number, target_url], |row| row.get(0))
        .map_err(|_| "authorization_control_lookup_failed".to_string())?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|_| "authorization_control_lookup_failed".to_string())?;
    Ok(AuthorizationControlSetup { attempt_number, identity_ids, contract_keys })
}

#[tauri::command]
pub fn get_authorization_control_setup(
    state: State<'_, AppState>, scan_id: String, target_url: String,
) -> Result<AuthorizationControlSetup, String> {
    let connection = db::open(&state.db_path)?;
    authorization_control_setup_in(&connection, &state.app_data_dir, &scan_id, &target_url)
}

fn authorization_control_urls(input: &SaveAuthorizationControlInput) -> Result<(String, String), String> {
    let target = reqwest::Url::parse(&input.target_url).map_err(|_| "authorization_target_invalid")?;
    let owner = reqwest::Url::parse(&input.owner_object_url).map_err(|_| "authorization_owner_url_invalid")?;
    let tester = reqwest::Url::parse(&input.tester_control_url).map_err(|_| "authorization_control_url_invalid")?;
    if !matches!(target.scheme(), "http" | "https")
        || owner.origin() != target.origin()
        || tester.origin() != target.origin()
        || owner.path() != tester.path()
        || owner.path() == "/"
        || [target, owner.clone(), tester.clone()].iter().any(|url| {
            !url.username().is_empty() || url.password().is_some() || url.fragment().is_some()
        })
    {
        return Err("authorization_control_scope_invalid".into());
    }
    let query_without_selector = |url: &reqwest::Url, expected: &str| -> Result<Vec<(String, String)>, String> {
        let mut selector_count = 0;
        let mut rest = Vec::new();
        for (key, value) in url.query_pairs() {
            if key == input.object_query_key {
                selector_count += 1;
                if value != expected {
                    return Err("authorization_object_selector_mismatch".into());
                }
            } else {
                rest.push((key.into_owned(), value.into_owned()));
            }
        }
        if selector_count != 1 {
            return Err("authorization_object_selector_missing_or_ambiguous".into());
        }
        rest.sort();
        Ok(rest)
    };
    if query_without_selector(&owner, &input.owner_object_value)?
        != query_without_selector(&tester, &input.tester_object_value)?
    {
        return Err("authorization_control_request_shape_mismatch".into());
    }
    Ok((owner.to_string(), tester.to_string()))
}

fn valid_authorization_pointer(pointer: &str) -> bool {
    if !pointer.starts_with('/') || pointer.len() > 240 {
        return false;
    }
    pointer.split('/').skip(1).all(|segment| {
        !segment.is_empty()
            && segment != "-"
            && {
                let mut chars = segment.chars();
                let mut valid = true;
                while let Some(character) = chars.next() {
                    if character == '~' && !matches!(chars.next(), Some('0' | '1')) {
                        valid = false;
                        break;
                    }
                }
                valid
            }
    })
}

fn save_authorization_control_in(
    connection: &mut rusqlite::Connection,
    app_data_dir: &Path,
    input: &SaveAuthorizationControlInput,
) -> Result<(), String> {
    if input.scan_id.trim().is_empty()
        || matches!(input.scan_id.as_str(), "." | "..")
        || input.scan_id.contains(['/', '\\'])
        || input.attempt_number <= 0
        || input.contract_key.trim().is_empty()
        || input.contract_key.len() > 240
        || input.object_query_key.trim().is_empty()
        || input.object_query_key.len() > 80
        || input.owner_object_value.trim().is_empty()
        || input.tester_object_value.trim().is_empty()
        || input.owner_object_value == input.tester_object_value
        || input.owner_identity.trim().is_empty()
        || input.tester_identity.trim().is_empty()
        || input.owner_identity == input.tester_identity
        || !valid_authorization_pointer(&input.response_object_pointer)
    {
        return Err("authorization_control_invalid".into());
    }
    let (owner_url, tester_url) = authorization_control_urls(input)?;
    // Serialize registration against scan state changes. A prepared control
    // belongs to the *next* attempt: draft scans have not incremented
    // attempt_count yet, and the work-directory planner may skip old slots.
    let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| format!("authorization_control_transaction_failed: {error}"))?;
    let valid_target: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_targets WHERE scan_id=?1 AND url=?2)",
        params![input.scan_id, input.target_url],
        |row| row.get(0),
    ).map_err(|error| format!("authorization_target_lookup_failed: {error}"))?;
    let attempt: Option<(i64, String)> = transaction.query_row(
        "SELECT attempt_count,status FROM sentinel_scans WHERE id=?1 AND scan_type='web'",
        [&input.scan_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional().map_err(|error| format!("authorization_scan_lookup_failed: {error}"))?;
    let planned_attempt = attempt.as_ref().map(|(count, _)| {
        next_scan_attempt_number(
            &neutral_scan_work_root(app_data_dir).join(&input.scan_id),
            count.saturating_add(1).clamp(1, i64::from(u32::MAX)) as u32,
        ) as i64
    });
    if !valid_target || attempt.as_ref().map(|(_, status)| status.as_str()) != Some("draft")
        || planned_attempt != Some(input.attempt_number)
    {
        return Err("authorization_control_task_binding_invalid".into());
    }
    let (_, identities) = crate::auth_session::validated_scan_identities(
        &transaction, &input.scan_id, &input.target_url,
    )?;
    if !identities.contains(&input.owner_identity)
        || !identities.contains(&input.tester_identity)
    {
        return Err("authorization_control_identity_unbound".into());
    }
    let started: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3)",
        params![input.scan_id, input.attempt_number, input.target_url],
        |row| row.get(0),
    ).map_err(|error| format!("authorization_control_run_lookup_failed: {error}"))?;
    if started {
        return Err("authorization_control_attempt_already_started".into());
    }
    let (group_count, already_registered): (i64, i64) = transaction.query_row(
        "SELECT COUNT(*),COUNT(CASE WHEN contract_key=?4 THEN 1 END) \
         FROM agent_authorization_controls WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3",
        params![input.scan_id, input.attempt_number, input.target_url, input.contract_key],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).map_err(|_| "authorization_control_limit_lookup_failed".to_string())?;
    if group_count >= 4 && already_registered == 0 {
        return Err("authorization_control_group_limit_exceeded".into());
    }
    let inserted = transaction.execute(
        "INSERT OR IGNORE INTO agent_authorization_controls(\
         scan_id,attempt_number,target_url,contract_key,method,owner_object_url,tester_control_url,\
         object_query_key,owner_object_value,tester_object_value,response_object_pointer,owner_identity,tester_identity)\
         VALUES(?1,?2,?3,?4,'GET',?5,?6,?7,?8,?9,?10,?11,?12)",
        params![input.scan_id,input.attempt_number,input.target_url,input.contract_key,owner_url,tester_url,
            input.object_query_key,input.owner_object_value,input.tester_object_value,input.response_object_pointer,
            input.owner_identity,input.tester_identity],
    ).map_err(|error| format!("authorization_control_store_failed: {error}"))?;
    if inserted == 0 {
        return Err("authorization_control_immutable_conflict".into());
    }
    transaction.commit().map_err(|error| format!("authorization_control_commit_failed: {error}"))
}

#[tauri::command]
pub fn save_authorization_control(
    state: State<'_, AppState>,
    input: SaveAuthorizationControlInput,
) -> Result<(), String> {
    let mut connection = db::open(&state.db_path)?;
    save_authorization_control_in(&mut connection, &state.app_data_dir, &input)
}
