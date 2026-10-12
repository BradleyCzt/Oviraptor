//! Offline capability-package verification and opaque cache staging. This
//! does not approve, extract, or execute a package. In particular, a successful
//! check is not evidence of a sandbox or of a publisher approved by this app.
#![allow(dead_code)] // No administrator trust store or public import command yet.

use aws_lc_rs::signature::{UnparsedPublicKey, ED25519};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

const SIGNING_DOMAIN: &[u8] = b"OVIRAPTOR-TOOL-MANIFEST-V1\0";
const MAX_MANIFEST_BYTES: usize = 16 * 1024;
// This in-memory staging path is only for small worker packages; a browser
// distribution requires a separate bounded streaming importer.
const MAX_OPAQUE_PACKAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_STREAMED_PACKAGE_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ToolCapabilityManifest {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    pub platform: String,
    pub source: String,
    pub digest: String,
    pub license: String,
    pub entrypoint: String,
    pub broker_capabilities: Vec<String>,
    pub network_policy: String,
    pub filesystem_policy: String,
    pub timeout_seconds: u32,
    pub memory_mib: u32,
    pub cpu_limit: u32,
    pub process_limit: u32,
    pub output_bytes: u32,
    pub input_schema_version: u32,
    pub output_schema_version: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SignedToolManifest {
    pub payload: ToolCapabilityManifest,
    pub signer_id: String,
    pub signature_base64: String,
}

/// Trust anchors must be supplied by an administrator-controlled, separately
/// authenticated store. Neither the package nor the Agent may supply one.
pub(crate) struct ToolTrustAnchor {
    pub signer_id: String,
    pub public_key: Vec<u8>,
    pub revoked: bool,
}

fn signed_message(payload: &ToolCapabilityManifest) -> Result<Vec<u8>, String> {
    let mut message = SIGNING_DOMAIN.to_vec();
    message.extend(serde_json::to_vec(payload).map_err(|_| "tool_manifest_encoding_failed")?);
    Ok(message)
}

fn valid_entrypoint(entrypoint: &str) -> bool {
    if entrypoint.is_empty()
        || entrypoint.len() > 240
        || entrypoint.contains('\\')
        || entrypoint.contains(':')
        || !entrypoint
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'/' | b'.' | b'_' | b'-'))
    {
        return false;
    }
    let path = Path::new(entrypoint);
    !path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        && entrypoint
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == b'-' || ch == b'_')
}

/// A candidate's identity includes the signed release and target platform,
/// not just the bytes of its opaque package. Hashing the tuple avoids using
/// untrusted version/platform text as filesystem path components. Legacy
/// `id/digest` candidates are left untouched, but are not implicitly reused.
fn candidate_cache_key(payload: &ToolCapabilityManifest) -> Result<String, String> {
    let identity = serde_json::to_vec(&(
        &payload.id,
        &payload.version,
        &payload.platform,
        &payload.digest,
    ))
    .map_err(|_| "tool_manifest_encoding_failed")?;
    let mut hasher = Sha256::new();
    hasher.update(b"OVIRAPTOR-TOOL-CACHE-IDENTITY-V1\0");
    hasher.update(identity);
    Ok(format!("v1-{:x}", hasher.finalize()))
}

fn validate_payload(payload: &ToolCapabilityManifest, platform: &str) -> Result<(), String> {
    if payload.schema_version != 1
        || !valid_token(&payload.id)
        || payload.version.trim().is_empty()
        || payload.version.len() > 80
        || payload.version.eq_ignore_ascii_case("latest")
        || payload.platform != platform
        || payload.source.trim().is_empty()
        || payload.source.len() > 240
        || payload.license.trim().is_empty()
        || payload.license.len() > 80
        || !valid_entrypoint(&payload.entrypoint)
        || payload.input_schema_version != 1
        || payload.output_schema_version != 1
        || payload.timeout_seconds == 0
        || payload.timeout_seconds > 3_600
        || payload.memory_mib == 0
        || payload.memory_mib > 16_384
        || payload.cpu_limit == 0
        || payload.cpu_limit > 16
        || payload.process_limit == 0
        || payload.process_limit > 256
        || payload.output_bytes == 0
        || payload.output_bytes > 32 * 1024 * 1024
        || payload.filesystem_policy != "read-only-input-and-private-temp-output"
        || !matches!(payload.network_policy.as_str(), "none" | "broker-only")
        || payload.broker_capabilities.is_empty()
        || payload.broker_capabilities.len() > 16
        || payload.broker_capabilities.iter().any(|capability| {
            !matches!(
                capability.as_str(),
                "evidence.read" | "browser_action" | "source_analysis"
            )
        })
        || payload
            .broker_capabilities
            .iter()
            .enumerate()
            .any(|(index, capability)| payload.broker_capabilities[..index].contains(capability))
        || (payload
            .broker_capabilities
            .iter()
            .any(|capability| capability == "browser_action")
            && payload.network_policy != "broker-only")
        || (payload
            .broker_capabilities
            .iter()
            .any(|capability| capability == "source_analysis")
            && payload.network_policy != "none")
    {
        return Err("tool_manifest_policy_invalid".into());
    }
    let hex = payload
        .digest
        .strip_prefix("sha256:")
        .ok_or("tool_manifest_digest_invalid")?;
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|ch| ch.is_ascii_digit() || (b'a'..=b'f').contains(&ch))
    {
        return Err("tool_manifest_digest_invalid".into());
    }
    Ok(())
}

/// This validates opaque package bytes before an independent safe extractor or
/// runner can use them. The caller must revalidate at atomic publication and
/// execution, because this function does not close the filesystem TOCTOU gap.
pub(crate) fn verify_staged_tool_blob(
    manifest_bytes: &[u8],
    package_bytes: &[u8],
    anchors: &[ToolTrustAnchor],
    platform: &str,
) -> Result<ToolCapabilityManifest, String> {
    if package_bytes.len() > MAX_OPAQUE_PACKAGE_BYTES {
        return Err("tool_package_too_large".into());
    }
    let payload = verify_signed_tool_manifest(manifest_bytes, anchors, platform)?;
    let digest = Sha256::digest(package_bytes);
    if format!("sha256:{digest:x}") != payload.digest {
        return Err("tool_package_digest_mismatch".into());
    }
    Ok(payload)
}

fn verify_signed_tool_manifest(
    manifest_bytes: &[u8],
    anchors: &[ToolTrustAnchor],
    platform: &str,
) -> Result<ToolCapabilityManifest, String> {
    if manifest_bytes.len() > MAX_MANIFEST_BYTES {
        return Err("tool_manifest_too_large".into());
    }
    let signed: SignedToolManifest =
        serde_json::from_slice(manifest_bytes).map_err(|_| "tool_manifest_invalid_json")?;
    validate_payload(&signed.payload, platform)?;
    if !valid_token(&signed.signer_id) {
        return Err("tool_signer_invalid".into());
    }
    let mut matching = anchors
        .iter()
        .filter(|anchor| anchor.signer_id == signed.signer_id);
    let anchor = matching
        .next()
        .ok_or("tool_signer_not_trusted_or_revoked")?;
    if matching.next().is_some() || anchor.revoked {
        return Err("tool_signer_not_trusted_or_revoked".into());
    }
    if anchor.public_key.len() != 32 {
        return Err("tool_signer_key_invalid".into());
    }
    let signature = base64::engine::general_purpose::STANDARD
        .decode(&signed.signature_base64)
        .map_err(|_| "tool_signature_invalid")?;
    if signature.len() != 64 {
        return Err("tool_signature_invalid".into());
    }
    UnparsedPublicKey::new(&ED25519, &anchor.public_key)
        .verify(&signed_message(&signed.payload)?, &signature)
        .map_err(|_| "tool_signature_invalid")?;
    Ok(signed.payload)
}

#[path = "tool_supply/cache.rs"]
mod cache;

#[path = "tool_supply/approval.rs"]
mod approval;
pub(crate) use approval::ToolAdministratorConfiguration;

#[path = "tool_supply/approval_writer.rs"]
mod approval_writer;

#[path = "tool_supply/frozen_plan.rs"]
pub(crate) mod frozen_plan;
pub(crate) mod execution_readiness;
pub(crate) mod package_format;
pub(crate) mod approved_package_source;
pub(crate) mod approved_package_publish;
pub(crate) mod approved_bundle;
pub(crate) mod approved_bundle_deploy;
#[cfg(test)]
mod approved_package_approval_tests;
#[cfg(all(test, target_os = "macos"))]
mod approved_bundle_deploy_review_tests;

#[path = "tool_supply/selection.rs"]
pub(crate) mod selection;

#[path = "tool_supply/administrator_service.rs"]
pub(crate) mod administrator_service;

#[path = "tool_supply/sandbox/macos_static_parser.rs"]
mod macos_static_parser;

#[path = "tool_supply/sandbox/container_parser.rs"]
pub(crate) mod container_parser;

#[cfg(test)]
#[path = "tool_supply/tests.rs"]
mod tests;
