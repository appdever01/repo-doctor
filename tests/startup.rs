mod common;
use common::{Fixture, finding};
use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn startup_helper() {
    let Ok(mode) = std::env::var("REPO_DOCTOR_STARTUP_HELPER") else {
        return;
    };
    fs::write("startup-parent.pid", std::process::id().to_string()).unwrap();
    let mut descendant = Command::new("/bin/sleep")
        .arg("60")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    fs::write("startup-descendant.pid", descendant.id().to_string()).unwrap();
    std::thread::spawn(move || descendant.wait());
    let secret = std::env::var("REPO_DOCTOR_STARTUP_SECRET").unwrap_or_default();
    println!("{secret}");
    eprintln!("{secret}");
    match mode.as_str() {
        "success" => (),
        "failure" => std::process::exit(7),
        "flood" => {
            std::io::stderr().write_all(&vec![b'x'; 20 * 1024]).unwrap();
            std::thread::sleep(Duration::from_secs(60));
        }
        "flood-exit" => {
            std::io::stdout().write_all(&vec![b'x'; 20 * 1024]).unwrap();
        }
        "tcp" | "brief-tcp" | "closed-tcp" => {
            let port: u16 = std::env::var("REPO_DOCTOR_STARTUP_PORT")
                .unwrap()
                .parse()
                .unwrap();
            let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
            listener.set_nonblocking(true).unwrap();
            let start = Instant::now();
            loop {
                match listener.accept() {
                    Ok((stream, _)) => drop(stream),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => (),
                    Err(error) => panic!("{error}"),
                }
                if mode != "tcp" && start.elapsed() > Duration::from_millis(300) {
                    drop(listener);
                    if mode == "closed-tcp" {
                        std::thread::sleep(Duration::from_secs(60));
                    }
                    std::process::exit(0);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        _ => std::thread::sleep(Duration::from_secs(60)),
    }
}

fn fixture(mode: &str, port: Option<u16>, timeout: u64) -> Fixture {
    let fixture = Fixture::new();
    let helper = std::env::current_exe().unwrap();
    fixture.executable("node", &format!("if [ \"$1\" = --version ]; then printf 'v22.1.0\\n'; else exec '{}' --exact startup_helper --nocapture --test-threads=1; fi", helper.display()));
    fixture.write("package.json", r#"{"engines":{"node":">=22"}}"#);
    fixture.write("app.js", "synthetic controlled startup entry point");
    let condition = port
        .map(|port| format!("mode='tcp'\naddress='127.0.0.1'\nport={port}"))
        .unwrap_or_else(|| "mode='exit'".into());
    fixture.write(".repo-doctor.toml", &format!("schema=1\n[startup]\ncommand=['node','app.js','SENTINEL_STARTUP_ARGUMENT']\n{condition}\ntimeout_seconds={timeout}\n"));
    fixture.write("helper-mode", mode);
    fixture
}

fn command(fixture: &Fixture, startup: bool) -> Command {
    let mut command = fixture.command();
    command
        .args(["--format", "json", "--strict"])
        .env(
            "REPO_DOCTOR_STARTUP_HELPER",
            fs::read_to_string(fixture.project.path().join("helper-mode")).unwrap(),
        )
        .env("REPO_DOCTOR_STARTUP_SECRET", "SENTINEL_STARTUP_SECRET");
    if startup {
        command.arg("--startup");
    }
    if let Ok(config) = fs::read_to_string(fixture.project.path().join(".repo-doctor.toml"))
        && let Some(port) = config.lines().find_map(|line| line.strip_prefix("port="))
    {
        command.env("REPO_DOCTOR_STARTUP_PORT", port);
    }
    command
}

fn report(fixture: &Fixture, startup: bool, code: i32) -> serde_json::Value {
    let output = command(fixture, startup).output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(code),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(output.stderr.is_empty());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("SENTINEL_STARTUP"));
    let value = serde_json::from_slice(&output.stdout).unwrap();
    common::validate(&value);
    value
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn dead(pid: i32) -> bool {
    let output = Command::new("/bin/ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let state = String::from_utf8_lossy(&output.stdout);
    state.trim().is_empty() || state.trim().starts_with('Z')
}

fn cleaned(fixture: &Fixture) {
    for name in ["startup-parent.pid", "startup-descendant.pid"] {
        let pid: i32 = fs::read_to_string(fixture.project.path().join(name))
            .unwrap_or_else(|error| panic!("Missing {name}: {error}"))
            .parse()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !dead(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(dead(pid), "Startup process {pid} survived");
    }
}

#[test]
fn startup_is_explicit_static_safe_and_requires_passing_setup() {
    let fixture = fixture("success", None, 2);
    finding(&report(&fixture, false, 1), "startup.disabled");
    let output = command(&fixture, true).arg("--static").output().unwrap();
    let value = serde_json::from_slice(&output.stdout).unwrap();
    finding(&value, "startup.disabled");
    assert!(!fixture.project.path().join("startup-parent.pid").exists());
    fixture.write(".env.example", "REPO_DOCTOR_STARTUP_REQUIRED=synthetic");
    finding(&report(&fixture, true, 1), "startup.prerequisite_unmet");
    assert!(!fixture.project.path().join("startup-parent.pid").exists());
    let empty = Fixture::new();
    finding(&empty.json(&["--startup"]).1, "input.config_invalid");
}

#[test]
fn zero_exit_nonzero_exit_and_output_limits_are_secret_safe_and_reaped() {
    for (mode, expected, exit) in [
        ("success", "startup.exit_success", 0),
        ("failure", "startup.exited", 1),
        ("flood", "startup.output_error", 1),
        ("flood-exit", "startup.output_error", 1),
        ("timeout", "startup.timeout", 1),
    ] {
        let fixture = fixture(mode, None, 1);
        finding(&report(&fixture, true, exit), expected);
        cleaned(&fixture);
    }
}

#[test]
fn tcp_readiness_requires_a_live_process_and_stable_listener_then_releases_it() {
    for (mode, expected, exit) in [
        ("tcp", "startup.tcp_ready", 0),
        ("brief-tcp", "startup.exited", 1),
        ("closed-tcp", "startup.timeout", 1),
        ("success", "startup.exited", 1),
    ] {
        let port = free_port();
        let fixture = fixture(mode, Some(port), 2);
        finding(&report(&fixture, true, exit), expected);
        cleaned(&fixture);
        assert!(TcpListener::bind(("127.0.0.1", port)).is_ok());
    }
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let fixture = fixture("tcp", Some(listener.local_addr().unwrap().port()), 2);
    finding(&report(&fixture, true, 1), "startup.port_occupied");
    assert!(!fixture.project.path().join("startup-parent.pid").exists());
    assert!(listener.local_addr().is_ok());
}

#[test]
fn cancellation_reaps_parent_and_descendant_and_preserves_setup_findings() {
    use nix::{
        sys::signal::{Signal, kill},
        unistd::Pid,
    };
    for signal in [Signal::SIGINT, Signal::SIGTERM] {
        let fixture = fixture("timeout", None, 10);
        let mut child = command(&fixture, true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !fixture
            .project
            .path()
            .join("startup-descendant.pid")
            .exists()
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            fixture
                .project
                .path()
                .join("startup-descendant.pid")
                .exists()
        );
        kill(Pid::from_raw(child.id() as i32), signal).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(child.try_wait().unwrap().is_some());
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        let report = serde_json::from_slice(&output.stdout).unwrap();
        common::validate(&report);
        finding(&report, "startup.cancelled");
        finding(&report, "runtime.version_matches");
        cleaned(&fixture);
    }
}

#[test]
fn invalid_commands_approval_actions_and_bad_readiness_configs_never_execute() {
    let fixture = fixture("success", None, 2);
    for command in [
        "['sh','-c','SENTINEL_STARTUP_SECRET']",
        "['pnpm','approve-builds']",
        "['npm','install']",
        "['pnpm','run','approve-builds']",
        "['pnpm','run','start','--config.ignore-scripts=false']",
        "['node','../outside.js']",
        "['node','-e','process.exit(0)']",
    ] {
        fixture.write(
            ".repo-doctor.toml",
            &format!("schema=1\n[startup]\ncommand={command}\nmode='exit'"),
        );
        finding(&report(&fixture, true, 2), "input.config_invalid");
    }
    for condition in [
        "mode='tcp'\naddress='10.0.0.1'\nport=8000",
        "mode='tcp'\naddress='127.0.0.1'\nport=0",
        "mode='exit'\nport=8000",
        "mode='exit'\ntimeout_seconds=11",
        "mode='exit'\ntimeout_seconds=0",
        "mode='unknown'",
    ] {
        fixture.write(
            ".repo-doctor.toml",
            &format!("schema=1\n[startup]\ncommand=['node','app.js']\n{condition}"),
        );
        finding(&report(&fixture, true, 2), "input.config_invalid");
    }
    assert!(!fixture.project.path().join("startup-parent.pid").exists());
}

#[test]
fn missing_entrypoints_and_unsafe_links_remain_unverified() {
    let fixture = fixture("success", None, 2);
    fs::remove_file(fixture.project.path().join("app.js")).unwrap();
    finding(&report(&fixture, true, 1), "startup.entrypoint_missing");
    let external = tempfile::tempdir().unwrap();
    fs::write(external.path().join("entry.js"), "do not execute").unwrap();
    std::os::unix::fs::symlink(
        external.path().join("entry.js"),
        fixture.project.path().join("app.js"),
    )
    .unwrap();
    finding(&report(&fixture, true, 1), "startup.entrypoint_unverified");
    assert!(!fixture.project.path().join("startup-parent.pid").exists());
}

#[test]
fn human_output_and_startup_argument_bounds_remain_safe() {
    let fixture = fixture("failure", None, 2);
    let output = fixture
        .command()
        .arg("--startup")
        .env("REPO_DOCTOR_STARTUP_HELPER", "failure")
        .env("REPO_DOCTOR_STARTUP_SECRET", "SENTINEL_STARTUP_SECRET")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("exit code 7") && !text.contains("SENTINEL_STARTUP"));
    cleaned(&fixture);
    for args in [
        vec!["node".to_string(), "a".repeat(1025)],
        vec!["node".to_string(); 33],
        vec![
            "node".into(),
            "app.js".into(),
            "x".repeat(1024),
            "y".repeat(1024),
            "z".repeat(1024),
            "w".repeat(1024),
        ],
    ] {
        fixture.write(
            ".repo-doctor.toml",
            &format!(
                "schema=1\n[startup]\ncommand={}\nmode='exit'",
                serde_json::to_string(&args).unwrap()
            ),
        );
        finding(&report(&fixture, true, 2), "input.config_invalid");
    }
}

#[tokio::test]
async fn the_whole_run_budget_cannot_be_extended_by_startup() {
    use repo_doctor::{checks::startup, config::Config, runner::Host, safety::FileReader};
    use std::sync::Arc;
    let fixture = Fixture::new();
    fixture.write("app.js", "synthetic");
    fixture.executable("node", "printf '%s' \"$$\" > startup-parent.pid\n/bin/sleep 60 &\nprintf '%s' \"$!\" > startup-descendant.pid\nwait");
    let root = fixture.project.path().canonicalize().unwrap();
    let mut reader = FileReader::new(root.clone());
    let mut host = Host {
        root,
        path: Some(fixture.host.path().as_os_str().to_owned()),
        deadline: Instant::now() + Duration::from_secs(2),
        cancelled: false,
        filesystem_cancelled: Arc::default(),
    };
    let config = Config::parse(
        "schema=1\n[startup]\ncommand=['node','app.js']\nmode='exit'\ntimeout_seconds=10",
    )
    .unwrap();
    let start = Instant::now();
    let result = startup::check(
        config.startup.as_ref().unwrap(),
        true,
        true,
        &mut reader,
        &mut host,
    )
    .await;
    assert_eq!(result.finding_code, "startup.deadline");
    assert!(start.elapsed() < Duration::from_secs(3));
    cleaned(&fixture);
}

#[test]
fn startup_launchers_follow_trusted_path_selection_without_fallback() {
    let fixture = fixture("success", None, 2);
    fixture.write(
        ".repo-doctor.toml",
        "schema=1\n[startup]\ncommand=['python3','app.js']\nmode='exit'",
    );
    finding(&report(&fixture, true, 1), "startup.launcher_missing");
    fixture.executable("python3", "touch should-never-run");
    fs::remove_file(fixture.host.path().join("python3")).unwrap();
    let project_binary = fixture.project.path().join("python3");
    common::executable(&project_binary, "touch should-never-run");
    std::os::unix::fs::symlink(project_binary, fixture.host.path().join("python3")).unwrap();
    finding(&report(&fixture, true, 1), "startup.launcher_unverified");
    assert!(!fixture.project.path().join("should-never-run").exists());
}

#[test]
fn package_manager_startup_forces_script_and_download_policy() {
    let fixture = fixture("success", None, 2);
    fixture.executable("pnpm", "test \"$1\" = --config.ignore-scripts=true || exit 8\ntest \"$2\" = --config.enable-pre-post-scripts=false || exit 8\ntest \"$3\" = --config.manage-package-manager-versions=false || exit 8\ntest \"$4\" = --config.pm-on-fail=error || exit 8\ntest \"$5\" = --config.runtime-on-fail=error || exit 8\ntest \"$6\" = --config.use-node-version= || exit 8\ntest \"$7\" = --config.verify-deps-before-run=false || exit 8\ntest \"$8\" = run || exit 8\ntest \"$9\" = dev || exit 8\ntest \"$COREPACK_ENABLE_NETWORK\" = 0 || exit 8\ntest \"$npm_config_ignore_scripts\" = true || exit 8\nexit 0");
    fixture.write(
        ".repo-doctor.toml",
        "schema=1\n[startup]\ncommand=['pnpm','run','dev']\nmode='exit'",
    );
    finding(&report(&fixture, true, 0), "startup.exit_success");
}
