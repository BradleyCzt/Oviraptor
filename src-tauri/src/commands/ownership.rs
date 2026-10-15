//! Asset ownership: profiles, organization directory, review groups, durable decisions,
//! authorization and read-only unit projection. Exposure consumes the explicit read API;
//! scan/model execution remains outside this domain.
use crate::{
    db,
    asset_models::{
        AssetOwnershipAssessmentResult, AssetOwnershipDecisionInput, AssetOwnershipProfile,
        AssetOwnershipSummary, AssetSelection,
    },
    AppState,
};
use rusqlite::{params, params_from_iter, types::Value as SqlValue, OptionalExtension, Row};
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::OnceLock,
    thread,
    time::{Duration, Instant},
};
use tauri::State;
use uuid::Uuid;

#[cfg(test)]
use crate::asset_models::AssetQuery;
#[cfg(test)]
use std::fs;
#[cfg(test)]
use super::{assets::asset_filter, authorization_control_setup_in};

// These source units share only the cohesive ownership domain's explicit scope.
include!("asset_ownership.rs");
include!("ownership_directory.rs");
include!("ownership_public_suffix.rs");
include!("ownership_review.rs");
include!("ownership_operations.rs");
include!("ownership_authorization.rs");
include!("ownership_unit_projection.rs");
include!("ownership_o3_tests.rs");
