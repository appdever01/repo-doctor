use crate::safety::{Cancellation, OUTPUT_LIMIT};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

pub enum Selection {
    Trusted(PathBuf),
    Missing,
    Unverified(&'static str),
}

pub enum Observation {
    Version(String),
    Missing,
    Unverified(&'static str),
    Error(&'static str),
}

pub struct Host {
    pub root: PathBuf,
    pub path: Option<OsString>,
    pub deadline: Instant,
    pub cancelled: bool,
    pub filesystem_cancelled: Arc<Cancellation>,
}

struct ProcessGroup(Option<u32>);

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0 {
            use nix::{
                sys::signal::{Signal, killpg},
                unistd::Pid,
            };
            let _ = killpg(Pid::from_raw(pid as i32), Signal::SIGKILL);
        }
    }
}

pub fn select_executable(name: &str, path: Option<&std::ffi::OsStr>, root: &Path) -> Selection {
    let Some(path) = path else {
        return Selection::Missing;
    };
    for directory in std::env::split_paths(path) {
        let candidate = directory.join(name);
        let Ok(metadata) = candidate.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                continue;
            }
        }
        if !directory.is_absolute() {
            return Selection::Unverified("PATH selects a relative entry.");
        }
        let Ok(real) = candidate.canonicalize() else {
            return Selection::Unverified("The selected executable could not be resolved.");
        };
        if real.starts_with(root) || candidate.starts_with(root) {
            return Selection::Unverified("PATH selects an executable inside the project.");
        }
        if candidate
            .components()
            .chain(real.components())
            .any(|c| c.as_os_str() == "shims")
        {
            return Selection::Unverified("PATH selects an unresolved version-manager shim.");
        }
        return Selection::Trusted(real);
    }
    Selection::Missing
}

async fn bounded_read<R: tokio::io::AsyncRead + Unpin>(stream: R) -> Result<Vec<u8>, &'static str> {
    let mut bytes = Vec::with_capacity(OUTPUT_LIMIT + 1);
    stream
        .take((OUTPUT_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| "Runtime output could not be read.")?;
    if bytes.len() > OUTPUT_LIMIT {
        return Err("Runtime output exceeded 16 KiB per stream.");
    }
    Ok(bytes)
}

pub async fn cancellation() {
    #[cfg(unix)]
    if let Ok(mut terminate) =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => (),
            _ = terminate.recv() => (),
        }
        return;
    }
    let _ = tokio::signal::ctrl_c().await;
}

pub async fn probe(
    executable: &Path,
    args: &[&str],
    budget: Duration,
) -> Result<String, &'static str> {
    let mut command = Command::new(executable);
    command
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|_| "The selected runtime could not be executed.")?;
    let group = ProcessGroup(child.id());
    let stdout = child
        .stdout
        .take()
        .ok_or("Runtime stdout is unavailable.")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("Runtime stderr is unavailable.")?;
    let result = tokio::select! {
        result = tokio::time::timeout(budget, async {
            let (out, err, status) = tokio::try_join!(bounded_read(stdout), bounded_read(stderr), async { child.wait().await.map_err(|_| "Runtime status could not be read.") })?;
            if !status.success() { return Err("Runtime exited with a nonzero status."); }
            let bytes = if out.is_empty() { err } else { out };
            String::from_utf8(bytes).map_err(|_| "Runtime output is not valid UTF-8.")
        }) => result.unwrap_or(Err("Runtime probe exceeded its deadline.")),
        _ = cancellation() => Err("The run was cancelled."),
    };
    drop(group);
    if result.is_err() {
        let _ = child.kill().await;
    }
    let _ = child.wait().await;
    result
}

impl Host {
    pub async fn select(&self, name: &str) -> Selection {
        let name = name.to_owned();
        let root = self.root.clone();
        let path = self.path.clone();
        crate::safety::within_deadline(self.deadline, &self.filesystem_cancelled, move || {
            select_executable(&name, path.as_deref(), &root)
        })
        .await
        .unwrap_or_else(Selection::Unverified)
    }

    pub async fn runtime(&mut self, kind: &str) -> Observation {
        let names: &[&str] = if kind == "node" {
            &["node"]
        } else {
            &["python3", "python"]
        };
        let mut last = Observation::Missing;
        for name in names {
            if self.cancelled
                || self.filesystem_cancelled.is_cancelled()
                || Instant::now() >= self.deadline
            {
                return Observation::Unverified(
                    "The run was cancelled or exceeded its whole-run deadline.",
                );
            }
            let selected = self.select(name).await;
            let executable = match selected {
                Selection::Missing => continue,
                Selection::Unverified(reason) => return Observation::Unverified(reason),
                Selection::Trusted(path) => path,
            };
            let budget =
                Duration::from_secs(3).min(self.deadline.saturating_duration_since(Instant::now()));
            let args: &[&str] = if kind == "node" {
                &["--version"]
            } else {
                &["-I", "--version"]
            };
            match probe(&executable, args, budget).await {
                Ok(output) => {
                    let value = output.trim();
                    if kind == "node" {
                        if let Some(numeric) = value.strip_prefix('v') {
                            let parts: Vec<_> = numeric.split('.').collect();
                            if parts.len() == 3
                                && parts
                                    .iter()
                                    .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
                                && let Ok(version) = numeric.parse::<node_semver::Version>()
                            {
                                return Observation::Version(version.to_string());
                            }
                        }
                    } else if let Some(numeric) = value.strip_prefix("Python ")
                        && numeric.len() <= 64
                        && numeric
                            .bytes()
                            .all(|b| b.is_ascii_digit() || b".abrc".contains(&b))
                        && let Ok(version) = numeric.parse::<pep440_rs::Version>()
                    {
                        if version.release().first().is_some_and(|v| *v >= 3) {
                            return Observation::Version(version.to_string());
                        }
                        last = Observation::Error("Python 2 cannot satisfy a Python 3 project.");
                        continue;
                    }
                    last = Observation::Error(
                        "Runtime version output did not match its fixed format.",
                    );
                }
                Err(reason) => {
                    if reason == "The run was cancelled." {
                        self.cancelled = true;
                    }
                    last = Observation::Error(reason);
                }
            }
            if kind == "node" {
                break;
            }
        }
        last
    }
}
