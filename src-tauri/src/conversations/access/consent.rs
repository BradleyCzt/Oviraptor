//! Consent follows a recipient and exact selected guidance, rather than a UI boolean.
use super::{skills, snapshot, AccessSnapshot};
use crate::agent_runtime::model::GatewayProfile;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(super) fn identity(profile: &GatewayProfile) -> String {
    // Include the complete endpoint (including path/query), model and deployment.
    // Only this digest leaves the backend; keys and URL credentials never do.
    format!(
        "{:x}",
        Sha256::digest(
            format!("{}\0{}\0{}", profile.endpoint, profile.model, profile.local).as_bytes()
        )
    )
}
pub(super) fn recipient(db: &Connection) -> Option<(String, String)> {
    let profile = super::super::reply::resolve_profile(db).ok()?;
    let url = reqwest::Url::parse(&profile.endpoint).ok()?;
    let host = url.host_str()?;
    let label = format!(
        "{}://{}{}",
        url.scheme(),
        host,
        url.port()
            .map(|port| format!(":{port}"))
            .unwrap_or_default()
    );
    Some((identity(&profile), label))
}
pub(super) fn acknowledge(
    scope: &AccessSnapshot,
    provided: Option<&str>,
    skill_revision: Option<&str>,
) -> Result<String, String> {
    if skill_revision != Some(scope.skill_revision.as_str()) {
        return Err("conversation_access_skill_changed_review_required".into());
    }
    match (scope.provider_identity.as_deref(), provided) {
        (Some(actual), Some(provided)) if actual == provided => Ok(actual.into()),
        _ => Err("conversation_access_provider_changed_review_required".into()),
    }
}
pub(crate) struct FlightScope {
    revision: i64,
    recipient: Option<String>,
    skills: BTreeMap<i64, String>,
    grants_used: bool,
}
pub(crate) fn flight_scope(
    db: &Connection,
    project: i64,
    id: &str,
    expected: i64,
    profile: &GatewayProfile,
) -> Result<FlightScope, String> {
    let scope = snapshot(db, project, id)?;
    super::validate_scope_revision(db, project, id, expected)?;
    let grants_used = scope.agent_enabled && scope.provider_disclosure_accepted;
    if grants_used && scope.provider_identity.as_deref() != Some(identity(profile).as_str()) {
        return Err("conversation_access_provider_changed_review_required".into());
    }
    Ok(FlightScope {
        revision: expected,
        recipient: scope.provider_identity,
        skills: if grants_used {
            scope.skill_fingerprints
        } else {
            BTreeMap::new()
        },
        grants_used,
    })
}
pub(crate) fn validate_flight_scope(
    db: &Connection,
    project: i64,
    id: &str,
    expected: &FlightScope,
) -> Result<(), String> {
    let current = snapshot(db, project, id)?;
    if current.revision != expected.revision
        || current.provider_identity != expected.recipient
        || expected.grants_used && !current.provider_disclosure_accepted
    {
        return Err("conversation_access_changed_or_revoked".into());
    }
    for (id, hash) in &expected.skills {
        if skills::fingerprint(db, *id).as_ref() != Ok(hash) {
            return Err("conversation_access_skill_changed_or_revoked".into());
        }
    }
    Ok(())
}
