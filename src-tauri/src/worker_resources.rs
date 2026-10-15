//! Release-pinned worker resources and host-runtime discovery.
//! Integrity and Node selection are mechanisms; they confer no task authority
//! and do not claim process or network sandbox isolation.
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use tauri::{AppHandle, Manager};

pub(crate) fn resolve_frontend_recon_worker(app: &AppHandle) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("resources/workers/9_frontend_runtime_probe.cjs"));
        candidates.push(resource_dir.join("workers/9_frontend_runtime_probe.cjs"));
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/workers/9_frontend_runtime_probe.cjs"),
    );
    let worker = candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| "应用内置 CDP 运行时探测器 9_frontend_runtime_probe.cjs 缺失".to_string())?;
    verify_bundled_worker(&worker)?;
    Ok(worker)
}

pub(crate) fn sentinel_runtime_path(home: &Path) -> OsString {
    // Prefer Homebrew before /usr/local so Apple Silicon machines do not pick up an
    // ancient Intel-era Node (e.g. v16) that breaks CDP helper handshakes.
    #[cfg(not(windows))]
    let preferred = [
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".pyenv/shims"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
        PathBuf::from("/usr/sbin"),
        PathBuf::from("/sbin"),
    ];
    #[cfg(windows)]
    let preferred = [home.join(".pyenv/shims")];
    let mut paths: Vec<PathBuf> = preferred.to_vec();
    if let Some(existing) = std::env::var_os("PATH") {
        for path in std::env::split_paths(&existing) {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    std::env::join_paths(paths).unwrap_or_else(|_| std::env::var_os("PATH").unwrap_or_default())
}

// Enumerate without executing anything. Keep missing paths: installing a new
// candidate can change the resolver's winner even if PATH itself is unchanged.
pub(crate) fn helper_node_candidates(
    runtime_path: &std::ffi::OsStr,
    configured: Option<OsString>,
) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(configured) = configured {
        candidates.push(PathBuf::from(configured));
    }
    for dir in std::env::split_paths(runtime_path) {
        let candidate = dir.join(if cfg!(windows) { "node.exe" } else { "node" });
        candidates.push(candidate);
    }
    #[cfg(not(windows))]
    for path in [
        PathBuf::from("/opt/homebrew/bin/node"),
        PathBuf::from("/usr/local/bin/node"),
        PathBuf::from("/usr/bin/node"),
    ] {
        candidates.push(path);
    }
    let mut seen = HashSet::new();
    candidates.retain(|path| seen.insert(path.clone()));
    candidates
}

/// Pick a Node binary for CDP / AST helpers. Nest GUI PATH historically preferred
/// `/usr/local/bin/node` (often Node 16); Chrome's CDP pipe needs a modern Node.
pub(crate) fn resolve_helper_node(runtime_path: &OsString) -> Result<PathBuf, String> {
    let candidates: Vec<_> =
        helper_node_candidates(runtime_path, std::env::var_os("OVIRAPTOR_NODE_EXECUTABLE"))
            .into_iter()
            .filter(|path| path.is_file())
            .collect();
    if candidates.is_empty() {
        return Err("未找到可用的 Node.js；浏览器采集 helper 无法启动".into());
    }

    let mut scored: Vec<(u32, u32, u32, PathBuf, String)> = Vec::new();
    for path in candidates {
        let Ok(output) = Command::new(&path).arg("-v").output() else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let version = text.trim().trim_start_matches('v');
        let mut parts = version.split('.');
        let major = parts
            .next()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);
        let minor = parts
            .next()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);
        let patch = parts
            .next()
            .and_then(|v| v.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);
        scored.push((major, minor, patch, path, format!("v{version}")));
    }
    if scored.is_empty() {
        return Err("Node.js 存在但无法读取版本".into());
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(b.2.cmp(&a.2)));
    if let Some((_, _, _, path, _)) = scored.iter().find(|(major, _, _, _, _)| *major >= 18) {
        return Ok(path.clone());
    }
    let (major, _, _, path, version) = &scored[0];
    Err(format!(
        "浏览器采集需要 Node.js 18+，当前最新可用为 {}（{}，主版本 {major}）",
        path.display(),
        version
    ))
}

/// The shipped JavaScript is part of the application release, not a mutable
/// tool installed from the host PATH. Compare against bytes compiled into this
/// release; a missing or modified resource is an unavailable capability.
/// This is integrity checking, not a process/network sandbox or a signature
/// for separately downloaded capability bundles.
pub(crate) fn verify_bundled_worker(helper: &Path) -> Result<(), String> {
    fn verify_file(
        path: &Path,
        expected: &[u8],
        missing: &str,
        mismatch: &str,
    ) -> Result<(), String> {
        let metadata = fs::symlink_metadata(path).map_err(|_| missing.to_string())?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(mismatch.into());
        }
        let actual = fs::read(path).map_err(|_| mismatch.to_string())?;
        if Sha256::digest(&actual) != Sha256::digest(expected) {
            return Err(mismatch.into());
        }
        Ok(())
    }

    match helper.file_name().and_then(|name| name.to_str()) {
        Some("9_frontend_runtime_probe.cjs") => {
            verify_file(
                helper,
                include_bytes!("../resources/workers/9_frontend_runtime_probe.cjs"),
                "bundled_worker_missing",
                "bundled_worker_digest_mismatch",
            )?;
            let config = helper
                .parent()
                .and_then(Path::parent)
                .ok_or("bundled_worker_browser_locations_missing")?
                .join("config/browser-locations.json");
            verify_file(
                &config,
                include_bytes!("../resources/config/browser-locations.json"),
                "bundled_worker_browser_locations_missing",
                "bundled_worker_browser_locations_mismatch",
            )
        }
        Some("8_js_ast_analyzer.cjs") => {
            verify_file(
                helper,
                include_bytes!("../resources/workers/8_js_ast_analyzer.cjs"),
                "bundled_worker_missing",
                "bundled_worker_digest_mismatch",
            )?;
            verify_file(
                &helper.with_file_name("babel-parser.cjs"),
                include_bytes!("../resources/workers/babel-parser.cjs"),
                "bundled_worker_parser_missing",
                "bundled_worker_parser_digest_mismatch",
            )
        }
        _ => Err("bundled_worker_unregistered".into()),
    }
}
