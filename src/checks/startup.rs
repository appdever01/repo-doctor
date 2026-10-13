use crate::config::{Startup, StartupMode};
use crate::model::{CheckResult, Status};
use crate::runner::{Host, ProcessGroup, Selection};
use crate::safety::{FileReader, OUTPUT_LIMIT};
use std::net::{SocketAddr, TcpListener};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

fn result(status: Status, code: &str, message: impl Into<String>) -> CheckResult {
    CheckResult::new("startup", status, code, message, ".repo-doctor.toml")
        .next("Review the configured startup command outside Repo Doctor. Application logs and arguments are not included; no scripts were approved.")
}

async fn drain<R: tokio::io::AsyncRead + Unpin>(mut stream: R) -> Result<(), &'static str> {
    let mut total = 0;
    let mut buffer = [0; 2048];
    loop {
        let bytes = stream
            .read(&mut buffer)
            .await
            .map_err(|_| "Startup output could not be read.")?;
        if bytes == 0 {
            return Ok(());
        }
        total += bytes;
        if total > OUTPUT_LIMIT {
            return Err(
                "Startup output exceeded 16 KiB per stream; all application output was discarded.",
            );
        }
    }
}

async fn observe(
    child: &mut Child,
    config: &Startup,
    host: &Host,
    deadline: Instant,
    shortened: bool,
) -> CheckResult {
    let Some(stdout) = child.stdout.take() else {
        return result(
            Status::Error,
            "startup.inspection_error",
            "Startup stdout was unavailable.",
        );
    };
    let Some(stderr) = child.stderr.take() else {
        return result(
            Status::Error,
            "startup.inspection_error",
            "Startup stderr was unavailable.",
        );
    };
    let output = async { tokio::try_join!(drain(stdout), drain(stderr)) };
    tokio::pin!(output);
    let mut drained = false;
    let mut ready_since = None;
    let mut tcp_ready = false;
    let mut exit_success = false;
    loop {
        tokio::select! {
            biased;
            _ = host.filesystem_cancelled.wait() => return result(Status::Error, "startup.cancelled", "The startup check was cancelled."),
            _ = tokio::time::sleep_until(deadline.into()) => return if shortened {
                result(Status::Error, "startup.deadline", "The whole-run budget ended before startup could be established.")
            } else {
                result(Status::Fail, "startup.timeout", "The configured startup condition was not established within its timeout.")
            },
            observed = &mut output, if !drained => {
                match observed {
                    Ok(_) => drained = true,
                    Err(reason) => return result(Status::Error, "startup.output_error", reason),
                }
            }
            _ = std::future::ready(()), if tcp_ready || (exit_success && drained) => {
                if exit_success {
                    return result(Status::Pass, "startup.exit_success", "The configured application command exited successfully; this does not establish server readiness.");
                }
                if matches!(child.try_wait(), Ok(None)) {
                    return result(Status::Pass, "startup.tcp_ready", "The foreground process stayed running while successive loopback TCP probes succeeded for one second; listener ownership, HTTP and application health were not verified.");
                }
                tcp_ready = false;
            }
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                match child.try_wait() {
                    Ok(Some(status)) if config.mode == StartupMode::Exit && status.success() => {
                        exit_success = true;
                    }
                    Ok(Some(status)) => {
                        let detail = status.code().map(|code| format!("exit code {code}")).unwrap_or_else(|| "a termination signal".into());
                        return result(Status::Fail, "startup.exited", format!("The application ended with {detail} before satisfying the configured startup condition."));
                    }
                    Ok(None) => (),
                    Err(_) => return result(Status::Error, "startup.inspection_error", "The startup process status could not be inspected."),
                }
                if let (Some(address), Some(port)) = (config.address, config.port) {
                    let accepted = tokio::select! {
                        _ = host.filesystem_cancelled.wait() => return result(Status::Error, "startup.cancelled", "The startup check was cancelled."),
                        connection = tokio::time::timeout(Duration::from_millis(100).min(deadline.saturating_duration_since(Instant::now())), tokio::net::TcpStream::connect(SocketAddr::new(address, port))) => matches!(connection, Ok(Ok(_))),
                    };
                    if accepted {
                        let since = ready_since.get_or_insert_with(Instant::now);
                        if since.elapsed() >= Duration::from_secs(1)
                            && matches!(child.try_wait(), Ok(None))
                            && Instant::now() < deadline
                            && !host.filesystem_cancelled.is_cancelled() {
                            tcp_ready = true;
                        }
                    } else {
                        ready_since = None;
                    }
                }
            }
        }
    }
}

pub async fn check(
    config: &Startup,
    enabled: bool,
    setup_passed: bool,
    reader: &mut FileReader,
    host: &mut Host,
) -> CheckResult {
    if !enabled {
        return result(
            Status::Skipped,
            "startup.disabled",
            "The configured startup command was not executed; enable --startup and omit --static to opt in.",
        );
    }
    if !setup_passed {
        return result(
            Status::Skipped,
            "startup.prerequisite_unmet",
            "Startup was not attempted because declared setup failed or remained incomplete.",
        );
    }
    if !cfg!(unix) {
        return result(
            Status::Skipped,
            "startup.platform_unverified",
            "Startup process-group cleanup is verified only on macOS and Linux.",
        );
    }
    if host.cancelled || host.filesystem_cancelled.is_cancelled() {
        return result(
            Status::Error,
            "startup.cancelled",
            "The startup check was cancelled before execution.",
        );
    }
    let executable = match host.select(&config.command[0]).await {
        Selection::Trusted(path) => path,
        Selection::Missing => {
            return result(
                Status::Fail,
                "startup.launcher_missing",
                "The configured host startup launcher is absent from PATH.",
            );
        }
        Selection::Unverified(reason) => {
            return result(Status::Error, "startup.launcher_unverified", reason);
        }
    };
    if matches!(config.command[0].as_str(), "node" | "python" | "python3") {
        match reader.present(&config.command[1]).await {
            Ok(true) => (),
            Ok(false) => {
                return result(
                    Status::Fail,
                    "startup.entrypoint_missing",
                    "The configured project entry-point file is absent.",
                );
            }
            Err(reason) => return result(Status::Error, "startup.entrypoint_unverified", reason),
        }
    }
    let address = config
        .address
        .zip(config.port)
        .map(|(ip, port)| SocketAddr::new(ip, port));
    if let Some(address) = address {
        match TcpListener::bind(address) {
            Ok(listener) => drop(listener),
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                return result(
                    Status::Fail,
                    "startup.port_occupied",
                    "The startup port was already occupied; no application was launched or existing process stopped.",
                );
            }
            Err(_) => {
                return result(
                    Status::Error,
                    "startup.port_unverified",
                    "Startup port availability could not be established; no application was launched.",
                );
            }
        }
    }
    let now = Instant::now();
    let remaining = host.deadline.saturating_duration_since(now);
    if remaining <= Duration::from_secs(1) || host.filesystem_cancelled.is_cancelled() {
        return result(
            Status::Error,
            "startup.budget_unavailable",
            "Insufficient whole-run budget remains to start and clean up the application.",
        );
    }
    let requested = Duration::from_secs(config.timeout_seconds);
    let budget = requested.min(remaining - Duration::from_secs(1));
    let deadline = now + budget;
    let mut command = Command::new(executable);
    if config.command[0] == "npm" {
        command.arg("--ignore-scripts");
    } else if config.command[0] == "pnpm" {
        command
            .args([
                "--config.ignore-scripts=true",
                "--config.enable-pre-post-scripts=false",
                "--config.manage-package-manager-versions=false",
                "--config.pm-on-fail=error",
                "--config.runtime-on-fail=error",
                "--config.use-node-version=",
                "--config.verify-deps-before-run=false",
            ])
            .env("npm_config_enable_pre_post_scripts", "false");
    }
    command
        .args(&config.command[1..])
        .current_dir(&host.root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env("COREPACK_ENABLE_NETWORK", "0")
        .env("npm_config_ignore_scripts", "true");
    for name in [
        "NODE_OPTIONS",
        "PYTHONPATH",
        "PYTHONSTARTUP",
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "DYLD_INSERT_LIBRARIES",
        "DYLD_LIBRARY_PATH",
        "DYLD_FALLBACK_LIBRARY_PATH",
    ] {
        command.env_remove(name);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => {
            return result(
                Status::Error,
                "startup.spawn_error",
                "The configured application launcher could not be started.",
            );
        }
    };
    let mut group = ProcessGroup(child.id());
    let observation = observe(&mut child, config, host, deadline, budget < requested).await;
    let killed = group.terminate();
    let _ = child.start_kill();
    let cleanup_budget =
        Duration::from_secs(1).min(host.deadline.saturating_duration_since(Instant::now()));
    let reaped = tokio::time::timeout(cleanup_budget, child.wait()).await;
    if killed.is_err() || !matches!(reaped, Ok(Ok(_))) {
        return result(
            Status::Error,
            "startup.cleanup_error",
            format!(
                "{} The startup process could not be fully terminated and reaped within the cleanup budget.",
                observation.message
            ),
        );
    }
    observation
}
