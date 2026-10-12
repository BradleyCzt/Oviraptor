//! Frozen frontend lookup only; no worker grant, transport, or audit writer.
use crate::agent_runtime::{
    multi_agent::{budget, lease::CoordinatorLease},
    secrets, store, web_mode,
};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub(crate) struct MapperLookupLimits {
    source_bytes: u64,
    observation_bytes: usize,
}
impl MapperLookupLimits {
    // These are new loop-birth limits, not inherited old Mapper ceilings.
    pub(crate) fn new(source_bytes: u64, observation_bytes: usize) -> Result<Self, String> {
        if source_bytes == 0
            || source_bytes > 256 * 1024
            || !(512..=24_000).contains(&observation_bytes)
        {
            return Err("mapper_lookup_limits_invalid".into());
        }
        Ok(Self {
            source_bytes,
            observation_bytes,
        })
    }
}
pub(crate) struct MapperEvidenceInventory {
    actor: CoordinatorLease,
    directory: PathBuf,
    original_mode: Value,
    fact: Value,
    limits: MapperLookupLimits,
}
pub(crate) struct MapperLookupObservation {
    value: Value,
}
impl MapperLookupObservation {
    pub(crate) fn value(&self) -> &Value {
        &self.value
    }
}
impl MapperEvidenceInventory {
    pub(crate) fn capture_on(
        db: &Connection,
        actor: &CoordinatorLease,
        target_dir: &Path,
        revision: i64,
        limits: MapperLookupLimits,
    ) -> Result<Self, String> {
        if db.is_autocommit() {
            return Err("mapper_lookup_snapshot_required".into());
        }
        if revision < 1 {
            return Err("mapper_lookup_revision_invalid".into());
        }
        budget::root::RootOwner::load_original(db, &actor.root_run_id)?
            .require_original_coordinator(db, actor)?;
        let original = super::original_root_capture_mode(db, &actor.root_run_id)?;
        web_mode::root::verify_birth(db, &actor.root_run_id, &original)?;
        let directory = super::checked_evidence_directory(
            Path::new(&original.fact().work_dir),
            target_dir,
            &actor.scan_id,
        )?;
        if directory != target_dir
            || super::resolve_agent_evidence_location(db, &actor.root_run_id)? != directory
        {
            return Err("mapper_lookup_directory_changed".into());
        }
        let bytes = read_original(&directory, limits.source_bytes)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| "mapper_lookup_source_invalid")?;
        let source: Value =
            serde_json::from_str(text).map_err(|_| "mapper_lookup_source_invalid")?;
        if !source.is_object()
            || source.get("error").is_some()
            || source["url"].as_str() != Some(actor.target_key.as_str())
        {
            return Err("mapper_lookup_source_scope_invalid".into());
        }
        let fact = json!({"schemaVersion":1,"rootRunId":actor.root_run_id,"scanId":actor.scan_id,"attemptNumber":actor.attempt_number,
            "targetKey":actor.target_key,"coordinatorEpoch":actor.lease_epoch,"coordinatorFence":actor.fencing_token,"evidenceRevision":revision,
            "limits":{"sourceMaxBytes":limits.source_bytes,"observationMaxBytes":limits.observation_bytes},
            "entries":[{"evidenceId":"frontend-evidence","artifact":"frontend-evidence.json","sha256":store::stable_hash(text),"sizeBytes":bytes.len(),"mediaType":"application/json"}]});
        let inventory = Self {
            actor: actor.clone(),
            directory,
            original_mode: serde_json::to_value(original.fact()).map_err(|e| e.to_string())?,
            fact,
            limits,
        };
        inventory.require_on(db, actor)?;
        Ok(inventory)
    }
    pub(crate) fn fact(&self) -> &Value {
        &self.fact
    }
    pub(crate) fn hash(&self) -> String {
        store::stable_hash(&self.fact.to_string())
    }
    pub(super) fn require_on(
        &self,
        db: &Connection,
        actor: &CoordinatorLease,
    ) -> Result<(), String> {
        if db.is_autocommit() {
            return Err("mapper_lookup_snapshot_required".into());
        }
        if actor != &self.actor {
            return Err("mapper_lookup_actor_changed".into());
        }
        budget::root::RootOwner::load_original(db, &actor.root_run_id)?
            .require_original_coordinator(db, actor)?;
        let original = super::original_root_capture_mode(db, &actor.root_run_id)?;
        web_mode::root::verify_birth(db, &actor.root_run_id, &original)?;
        if serde_json::to_value(original.fact()).map_err(|e| e.to_string())? != self.original_mode
            || super::checked_evidence_directory(
                Path::new(&original.fact().work_dir),
                &self.directory,
                &actor.scan_id,
            )? != self.directory
            || super::resolve_agent_evidence_location(db, &actor.root_run_id)? != self.directory
        {
            return Err("mapper_lookup_directory_or_birth_changed".into());
        }
        Ok(())
    }
    pub(crate) fn lookup_on(
        &self,
        db: &Connection,
        actor: &CoordinatorLease,
        args: &Value,
    ) -> Result<MapperLookupObservation, String> {
        self.require_on(db, actor)?;
        let bytes = read_original(&self.directory, self.limits.source_bytes)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| "mapper_lookup_source_invalid")?;
        if self.fact["entries"][0]["sha256"] != store::stable_hash(text)
            || self.fact["entries"][0]["sizeBytes"] != bytes.len()
        {
            return Err("mapper_lookup_source_changed".into());
        }
        let source: Value =
            serde_json::from_str(text).map_err(|_| "mapper_lookup_source_invalid")?;
        let schema_valid = args
            .as_object()
            .is_some_and(|o| o.len() == 1 && o.get("evidenceId").is_some_and(Value::is_string));
        let value = if !schema_valid {
            json!({"ok":false,"code":"mapper_lookup_arguments_invalid","executed":false})
        } else if args["evidenceId"] != "frontend-evidence" {
            json!({"ok":false,"code":"mapper_lookup_evidence_id_invalid","executed":false})
        } else {
            let redacted = secrets::redact_json(&source);
            let encoded = redacted.to_string();
            let mut end = encoded.len().min(self.limits.observation_bytes);
            while !encoded.is_char_boundary(end) {
                end -= 1;
            }
            loop {
                let observation = json!({"ok":true,"evidenceId":"frontend-evidence","sha256":self.fact["entries"][0]["sha256"],"sizeBytes":bytes.len(),
                    "mediaType":"application/json","preview":&encoded[..end],"truncated":end<encoded.len()});
                let size = observation.to_string().len();
                if size <= self.limits.observation_bytes {
                    break observation;
                }
                if end == 0 {
                    return Err("mapper_lookup_observation_limit_invalid".into());
                }
                end = end.saturating_sub(size - self.limits.observation_bytes);
                while !encoded.is_char_boundary(end) {
                    end -= 1;
                }
            }
        };
        // Drift and scope failures are stops even for invalid model arguments.
        // Only the stable deterministic schema/ID rejection may become an audit.
        let after = read_original(&self.directory, self.limits.source_bytes)?;
        if after != bytes {
            return Err("mapper_lookup_source_changed".into());
        }
        self.require_on(db, actor)?;
        Ok(MapperLookupObservation { value })
    }
}
fn read_original(directory: &Path, maximum: u64) -> Result<Vec<u8>, String> {
    // Existing Broker no-follow component walk and fixed-name openat prevent
    // parent symlink races, hardlinks, devices and model-selected paths.
    let handle =
        super::open_agent_artifact_directory(directory).ok_or("mapper_lookup_directory_invalid")?;
    let (bytes, _) = super::read_agent_artifact_file(&handle, "frontend-evidence.json", maximum)
        .ok_or("mapper_lookup_source_unreadable_or_limit")?;
    Ok(bytes)
}
