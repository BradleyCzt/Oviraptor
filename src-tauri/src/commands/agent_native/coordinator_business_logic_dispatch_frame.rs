// Independent paid prepare contract; output assessment never authorizes dispatch.
const NATIVE_BUSINESS_LOGIC_DISPATCH_RULE: &str = "original_paid_frozen_business_logic_dispatch_v1";

fn native_business_logic_dispatch_eligible(input: &VerifiedBusinessLogicInputCollection) -> bool {
    let evidence = &input.input()["frozenEvidence"];
    let Some(target) = input.task()["targetKey"]
        .as_str()
        .and_then(|s| reqwest::Url::parse(s).ok())
    else {
        return false;
    };
    // A captured same-origin POST form with a named input is a concrete flow
    // clue. Titles, prose, inferred API names and model hypotheses never count.
    evidence["forms"].as_array().is_some_and(|forms| {
        forms.iter().take(128).any(|form| {
            form["method"]
                .as_str()
                .is_some_and(|method| method.eq_ignore_ascii_case("POST"))
                && form["action"]
                    .as_str()
                    .and_then(|action| target.join(action).ok())
                    .is_some_and(|action| {
                        matches!(action.scheme(), "http" | "https")
                            && action.origin() == target.origin()
                            && action.username().is_empty()
                            && action.password().is_none()
                    })
                && form["inputs"].as_array().is_some_and(|inputs| {
                    inputs.iter().take(128).any(|field| {
                        field["name"].as_str().is_some_and(|name| {
                            !name.trim().is_empty() && name.chars().count() <= 128
                        })
                    })
                })
        })
    })
}

impl NativeCoordinatorFrame {
    fn business_logic_input_on(
        db: &rusqlite::Connection,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        target_dir: &Path,
    ) -> Result<Self, String> {
        if db.is_autocommit() {
            return Err("root_business_logic_dispatch_snapshot_required".into());
        }
        let original = original_root_capture_mode(db, &actor.root_run_id)?;
        crate::agent_runtime::web_mode::root::verify_birth(db, &actor.root_run_id, &original)?;
        let mode = crate::agent_runtime::web_mode::root::read(db, &actor.root_run_id)?
            .ok_or("root_business_logic_dispatch_protocol_missing")?;
        let bootstrap = mode.bootstrap_dispatch().unwrap_or(JsonValue::Null);
        if mode.fact() != original.fact()
            || mode.mode() != crate::agent_runtime::web_mode::WebMode::Multi
            || !matches!(bootstrap["schemaVersion"].as_i64(), Some(14 | 15))
            || bootstrap["businessLogicDispatchRule"] != NATIVE_BUSINESS_LOGIC_DISPATCH_RULE
            || bootstrap["businessLogicInputRule"] != BUSINESS_LOGIC_INPUT_RULE
            || bootstrap["businessLogicOutputRule"] != BUSINESS_LOGIC_OUTPUT_RULE
            || bootstrap["businessLogicOutputRequestCapacityRule"] != BUSINESS_LOGIC_CAPACITY_RULE
        {
            return Err("root_business_logic_dispatch_protocol_missing".into());
        }
        let input = native_business_logic_input_collection_on(db, actor, target_dir)?;
        if !native_business_logic_dispatch_eligible(&input) {
            return Err("root_business_logic_dispatch_not_eligible".into());
        }
        Ok(Self {
            kind: "business-logic-input",
            semantic: json!({"role":"business_logic","rootRunId":actor.root_run_id,"scanId":actor.scan_id,"attemptNumber":actor.attempt_number,"targetKey":actor.target_key,"originalInput":input.fact(),"eligibilityRule":"original_same_origin_post_form_named_input_v1","targetRequestsGranted":0,"dispatchGranted":false,"executionGrant":false}),
            rows: Vec::new(),
            review_refs: None,
            review_meta: None,
        })
    }

    fn verify_business_logic_dispatch_source(
        &self,
        db: &rusqlite::Connection,
        context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    ) -> Result<(), String> {
        if db.is_autocommit() {
            let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
            return self.verify_business_logic_dispatch_source(&tx, context, actor);
        }
        let original = Self::business_logic_input_on(db, actor, &context.target_dir)?;
        if self.fact() != original.fact()
            || self.review_refs.is_some()
            || self.review_meta.is_some()
        {
            return Err("root_business_logic_dispatch_original_input_changed".into());
        }
        native_coordinator_tick_authority(
            db,
            &native_coordinator_root_context(context, actor),
            actor,
        )
    }
}
