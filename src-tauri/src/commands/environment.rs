//! Operator runtime preparation. The durable preparation lease remains the
//! only fence for installs and explicit recovery; workers do not acquire tools.
use crate::{
    db,
    models::{EnvironmentDependency, EnvironmentReport},
    worker_resources::{resolve_frontend_recon_worker, verify_bundled_worker},
    AppState,
};
use std::{path::Path, process::Command};
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub fn check_environment(
    app: AppHandle,
    state: State<AppState>,
    profile_id: Option<i64>,
) -> Result<EnvironmentReport, String> {
    // Preserve the IPC input contract; current native helpers need no legacy executable settings.
    let _ = (&state, profile_id);
    let runtime_path = std::env::var_os("PATH").unwrap_or_default();
    let command_check = |command: &str, args: &[&str]| -> (bool, String) {
        match Command::new(command)
            .args(args)
            .env("PATH", &runtime_path)
            .output()
        {
            Ok(output) => {
                let mut text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if text.is_empty() {
                    text = String::from_utf8_lossy(&output.stderr).trim().to_string();
                }
                (
                    output.status.success(),
                    text.lines().next().unwrap_or("").to_string(),
                )
            }
            Err(error) => (false, error.to_string()),
        }
    };
    let mut node = "node".to_string();
    let mut node_ok = false;
    let mut node_version = String::new();
    for candidate in [
        "/opt/homebrew/bin/node",
        "/usr/local/bin/node",
        "node",
        "/usr/bin/node",
    ] {
        let (ok, version) = command_check(candidate, ["--version"].as_ref());
        if ok {
            node = candidate.to_string();
            node_ok = true;
            node_version = version;
            break;
        }
    }
    let mut dependencies = vec![EnvironmentDependency {
        name: "Oviraptor native workers".into(), command: "built-in Rust".into(),
        version: env!("CARGO_PKG_VERSION").into(), available: true,
        detail: "核心 Broker 随应用交付；浏览器/AST 脚本随包固定并校验完整性，但 Node/浏览器仍使用宿主运行时，尚未达到独立沙箱隔离".into(),
    }];
    let bundled_browser = resolve_frontend_recon_worker(&app);
    dependencies.push(EnvironmentDependency {
        name: "Bundled browser worker integrity".into(),
        command: "release-pinned JS".into(),
        version: if bundled_browser.is_ok() {
            env!("CARGO_PKG_VERSION")
        } else {
            "unavailable"
        }
        .into(),
        available: bundled_browser.is_ok(),
        detail: bundled_browser.as_ref().err().cloned().unwrap_or_else(|| {
            "随包浏览器脚本与当前应用版本一致；宿主 Node/浏览器及网络尚未隔离".into()
        }),
    });
    let bundled_ast = bundled_browser
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|browser| {
            verify_bundled_worker(&browser.with_file_name("8_js_ast_analyzer.cjs"))
        });
    dependencies.push(EnvironmentDependency {
        name: "Bundled AST and parser integrity".into(),
        command: "release-pinned JS".into(),
        version: if bundled_ast.is_ok() {
            env!("CARGO_PKG_VERSION")
        } else {
            "unavailable"
        }
        .into(),
        available: bundled_ast.is_ok(),
        detail: bundled_ast.err().unwrap_or_else(|| {
            "AST worker 和 Babel parser 均与当前应用版本一致；仍依赖宿主 Node".into()
        }),
    });
    dependencies.push(EnvironmentDependency {
        name: "Isolated browser/AST sandbox".into(),
        command: "sandbox adapter".into(),
        version: "unsupported_sandbox".into(),
        available: false,
        detail: "尚无经验证的独立进程/网络隔离及固定 Node/Chrome 能力包；脚本完整性不等于沙箱"
            .into(),
    });
    Ok(EnvironmentReport {
        os: if cfg!(target_os = "macos") {
            "macOS"
        } else if cfg!(target_os = "windows") {
            "Windows"
        } else {
            "Linux"
        }
        .into(),
        arch: std::env::consts::ARCH.into(),
        node: if node_ok {
            format!("{} · {}", node, node_version)
        } else {
            format!("不可用 · {}", node_version)
        },
        dependencies,
        checked_at: chrono::Utc::now().to_rfc3339(),
    })
}

struct EnvironmentInstallOutput<'a> {
    app: &'a AppHandle,
    journal: crate::installation_logs::Journal,
}

impl EnvironmentInstallOutput<'_> {
    fn record(&self, stage: &str, stream: &str, message: &str) -> Result<(), String> {
        let id = self.journal.append(stage, stream, message)?;
        // The event is only a wakeup. The committed journal row is the display
        // source of truth and remains available after listener failure.
        let _ = self
            .app
            .emit("environment-install-log", serde_json::json!({ "id": id }));
        Ok(())
    }
}

fn run_environment_install_step(
    output: &EnvironmentInstallOutput<'_>,
    stage: &str,
    description: &str,
    command: Command,
) -> Result<(), String> {
    output.record(stage, "status", &format!("开始：{description}"))?;
    let mut journal_error = None;
    let result = crate::installation_logs::run_command(command, |stream, line| {
        if journal_error.is_none() {
            journal_error = output.record(stage, stream, line).err();
        }
    });
    if let Some(error) = journal_error {
        return Err(error);
    }
    if let Err(error) = result {
        let message = crate::log_display::text(&format!("{description}失败：\n{error}"), 8000);
        output.record(stage, "error", &message)?;
        return Err(message);
    }
    output.record(stage, "success", &format!("完成：{description}"))?;
    Ok(())
}

/// Installation is an administrator preparation action, never a way for an
/// active task to acquire new tools. The durable preparation lease additionally
/// fences all scan activation transitions in the database.
pub(super) fn ensure_environment_install_idle(
    connection: &rusqlite::Connection,
) -> Result<(), String> {
    let active: i64 = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE status IN ('queued','scanning','pausing'))",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法确认任务状态，拒绝安装环境依赖：{error}"))?;
    if active != 0 {
        return Err(
            "存在等待或正在执行的扫描任务；请先完成或暂停并移出队列，再由管理员安装环境依赖".into(),
        );
    }
    Ok(())
}

fn install_windows_environment(
    output: &EnvironmentInstallOutput<'_>,
    _state: &AppState,
    _profile_id: Option<i64>,
) -> Result<String, String> {
    output.record(
        "prepare",
        "status",
        "开始安装 Windows Worker 基础环境；系统安装器如需授权会显示确认窗口",
    )?;
    let winget_available = Command::new("winget")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !winget_available {
        return Err(
            "未找到 winget。请从 Microsoft Store 安装“应用安装程序”，或按页面手动步骤安装依赖"
                .into(),
        );
    }
    for (stage, package, description) in [
        ("node", "OpenJS.NodeJS.LTS", "Node.js LTS"),
        ("tailscale", "Tailscale.Tailscale", "Tailscale"),
    ] {
        let mut command = Command::new("winget");
        command.args([
            "install",
            "--id",
            package,
            "--exact",
            "--accept-package-agreements",
            "--accept-source-agreements",
            "--disable-interactivity",
        ]);
        run_environment_install_step(
            output,
            stage,
            &format!("通过 winget 安装 {description}"),
            command,
        )?;
    }

    let result = "Windows Native Runtime 基础环境已安装；请登录 Tailscale 并重新执行环境检测";
    output.record("complete", "success", result)?;
    Ok(result.into())
}

#[tauri::command]
pub fn get_environment_preparation_status(
    state: State<AppState>,
) -> Result<db::EnvironmentPreparationStatus, String> {
    db::environment_preparation_status(&state.db_path)
}

#[tauri::command]
pub fn list_environment_install_logs(
    state: State<AppState>,
    after_id: Option<i64>,
    limit: Option<i64>,
) -> Result<crate::installation_logs::Page, String> {
    crate::installation_logs::page(&db::open(&state.db_path)?, after_id, limit)
}

#[tauri::command]
pub fn recover_environment_preparation(
    state: State<AppState>,
    expected_owner: String,
    acknowledgement: String,
) -> Result<(), String> {
    db::recover_environment_preparation(&state.db_path, &expected_owner, &acknowledgement)
}

#[tauri::command]
pub fn install_environment_dependencies(
    app: AppHandle,
    state: State<AppState>,
    profile_id: Option<i64>,
) -> Result<String, String> {
    if !cfg!(any(target_os = "windows", target_os = "macos")) {
        return Err("Linux 请按环境检测结果手动安装依赖，并配置 PATH".into());
    }
    let output = EnvironmentInstallOutput {
        app: &app,
        journal: crate::installation_logs::Journal::open(&state.db_path)?,
    };
    let preparation = db::begin_environment_preparation(&state.db_path)?;
    let result = install_environment_dependencies_while_locked(&output, &state, profile_id);
    match result {
        Ok(message) => {
            preparation.complete()?;
            Ok(message)
        }
        Err(error) => Err(format!(
            "{error}；环境准备租约保持锁定。请先核对所有安装子进程已停止，再到运行环境页人工恢复"
        )),
    }
}

fn install_environment_dependencies_while_locked(
    output: &EnvironmentInstallOutput<'_>,
    state: &AppState,
    profile_id: Option<i64>,
) -> Result<String, String> {
    ensure_environment_install_idle(&db::open(&state.db_path)?)?;
    if cfg!(target_os = "windows") {
        return install_windows_environment(output, state, profile_id);
    }
    output.record("prepare", "status", "开始检查并安装 Oviraptor 运行环境")?;
    let brew = ["/opt/homebrew/bin/brew", "/usr/local/bin/brew", "brew"]
        .into_iter()
        .find(|candidate| {
            Command::new(candidate)
                .arg("--version")
                .output()
                .is_ok_and(|output| output.status.success())
        })
        .map(str::to_string);
    let mut messages = Vec::new();
    let brew = match brew {
        Some(path) => {
            output.record("homebrew", "success", &format!("已找到 Homebrew：{path}"))?;
            path
        }
        None => {
            return Err("未找到 Homebrew；请管理员在扫描任务之外通过可信渠道安装，确认来源后重新运行环境安装。应用不会下载并执行远程安装脚本".into());
        }
    };
    let mut brew_install = Command::new(&brew);
    brew_install
        .args(["install", "node"])
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .env("HOMEBREW_NO_ENV_HINTS", "1");
    run_environment_install_step(output, "packages", "安装 Node.js", brew_install)?;
    messages.push("Node.js 安装完成".to_string());

    let tailscale_app = Path::new("/Applications/Tailscale.app");
    if !tailscale_app.exists() {
        let mut tailscale = Command::new(&brew);
        tailscale
            .args(["install", "--cask", "tailscale-app"])
            .env("HOMEBREW_NO_AUTO_UPDATE", "1")
            .env("HOMEBREW_NO_ENV_HINTS", "1");
        run_environment_install_step(output, "tailscale", "安装 Tailscale", tailscale)?;
        messages.push("Tailscale 安装完成（请打开并登录）".to_string());
    } else {
        output.record(
            "tailscale",
            "success",
            "Tailscale 已安装；Worker 启用前请确认已经登录 Tailnet",
        )?;
        messages.push("Tailscale 已存在".to_string());
    }

    let result = messages.join("；");
    output.record("complete", "success", &result)?;
    Ok(result)
}
