mod common;
use common::{Fixture, executable, finding};
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::time::{Duration, Instant};

#[test]
fn environment_precedence_optional_empty_and_duplicate_values() {
    let fixture = Fixture::new();
    fixture.write(
        ".repo-doctor.toml",
        "schema=1\n[env]\noptional=['OPTIONAL']\nrequired=['EXPLICIT']\n",
    );
    fixture.write(
        ".env.example",
        "REPO_DOCTOR_TEST_TOKEN=template-never-counts\nOPTIONAL=template\n",
    );
    fixture.write(
        ".env",
        "export REPO_DOCTOR_TEST_TOKEN='synthetic secret'\nEXPLICIT=valid\n",
    );
    fixture.write(
        ".env.local",
        "REPO_DOCTOR_TEST_TOKEN=first\nREPO_DOCTOR_TEST_TOKEN=\n",
    );
    let (_, report) = fixture.json(&[]);
    finding(&report, "env.empty");
    finding(&report, "env.duplicate");
    assert!(
        !report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == "env.OPTIONAL")
    );
    let output = fixture
        .command()
        .args(["--format", "json"])
        .env("REPO_DOCTOR_TEST_TOKEN", "synthetic-inherited")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["complete"], true);
    let output = fixture
        .command()
        .args(["--format", "json"])
        .env("EXPLICIT", "")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    fixture.write(".env.local", "REPO_DOCTOR_TEST_TOKEN=valid\n");
    assert_eq!(fixture.json(&[]).1["outcome"], "passed");
    fixture.write(
        ".repo-doctor.toml",
        "schema=1\n[env]\ntemplate='custom.example'\n",
    );
    finding(&fixture.json(&[]).1, "env.file_missing");
}

#[test]
fn sentinels_never_escape_into_reports_or_errors() {
    let fixture = Fixture::new();
    let secret = "DO_NOT_PRINT_SECRET_7e9e_001";
    fixture.write(".env.example", &format!("SAFE_NAME={secret}\n"));
    for text in [
        format!("SAFE_NAME={secret}\n"),
        format!("SAFE_NAME=\"{secret}\n"),
        format!("BAD-NAME={secret}\n"),
    ] {
        fixture.write(".env", &text);
        for format in ["json", "human"] {
            let output = fixture
                .command()
                .args(["--format", format])
                .output()
                .unwrap();
            for bytes in [&output.stdout, &output.stderr] {
                assert!(!String::from_utf8_lossy(bytes).contains(secret));
            }
        }
    }
    fixture.write("package.json", &format!("{{{secret}"));
    fixture.write("pyproject.toml", &format!("[{secret}"));
    fixture.write(".nvmrc", secret);
    fixture.executable("node", &format!("printf 'v22.1.0+{secret}\\n'"));
    for format in ["json", "human"] {
        let output = fixture
            .command()
            .args(["--format", format])
            .output()
            .unwrap();
        assert!(!String::from_utf8_lossy(&output.stdout).contains(secret));
        assert!(!String::from_utf8_lossy(&output.stderr).contains(secret));
    }
    fixture.write(
        ".repo-doctor.toml",
        &format!("schema=1\nunknown='{secret}'"),
    );
    let (_, report) = fixture.json(&[]);
    assert!(!report.to_string().contains(secret));
}

#[test]
fn project_local_executable_shims_relative_paths_and_injection_are_not_run() {
    let fixture = Fixture::new();
    fixture.write(
        "package.json",
        r#"{"engines":{"node":">=22"},"scripts":{"start":"touch sentinel"}}"#,
    );
    executable(&fixture.project.path().join("node"), "touch sentinel");
    fixture.executable("node", "printf 'v22.1.0\\n'");
    let output = fixture
        .command()
        .env(
            "PATH",
            std::env::join_paths([fixture.project.path(), fixture.host.path()]).unwrap(),
        )
        .args(["--format", "json"])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    finding(&report, "runtime.unverified");
    assert!(!fixture.project.path().join("sentinel").exists());
    let output = fixture
        .command()
        .current_dir(fixture.project.path())
        .env(
            "PATH",
            std::env::join_paths([std::path::Path::new("."), fixture.host.path()]).unwrap(),
        )
        .args(["--format", "json"])
        .output()
        .unwrap();
    finding(
        &serde_json::from_slice(&output.stdout).unwrap(),
        "runtime.unverified",
    );
    let shims = fixture.host.path().join("shims");
    fs::create_dir(&shims).unwrap();
    executable(&shims.join("node"), "touch sentinel");
    let output = fixture
        .command()
        .env("PATH", &shims)
        .args(["--format", "json"])
        .output()
        .unwrap();
    finding(
        &serde_json::from_slice(&output.stdout).unwrap(),
        "runtime.unverified",
    );
    fixture.executable("node", "if [ -n \"$NODE_OPTIONS$PYTHONPATH$DYLD_INSERT_LIBRARIES\" ]; then exit 9; fi\nprintf 'v22.1.0\\n'");
    let output = fixture
        .command()
        .env("NODE_OPTIONS", "--require malicious")
        .env("PYTHONPATH", "malicious")
        .env("DYLD_INSERT_LIBRARIES", "")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    fixture.write(".env.example", "SAFE_NAME=x\n");
    fixture.write(".env", "SAFE_NAME=$(touch sentinel)\n");
    fixture.json(&[]);
    assert!(!fixture.project.path().join("sentinel").exists());
}

#[tokio::test]
async fn file_boundaries_symlinks_permissions_and_limits() {
    let fixture = Fixture::new();
    fs::write(fixture.host.path().join("outside"), "{}").unwrap();
    symlink(
        fixture.host.path().join("outside"),
        fixture.project.path().join("package.json"),
    )
    .unwrap();
    finding(&fixture.json(&["--static"]).1, "file.read_error");
    fs::remove_file(fixture.project.path().join("package.json")).unwrap();
    fixture.write(
        ".repo-doctor.toml",
        "schema=1\n[env]\nfiles=['escape/missing']\n",
    );
    symlink(fixture.host.path(), fixture.project.path().join("escape")).unwrap();
    assert_eq!(fixture.json(&[]).1["exit_code"], 2);
    fs::remove_file(fixture.project.path().join(".repo-doctor.toml")).unwrap();
    fixture.write(
        "package.json",
        &"x".repeat(repo_doctor::safety::FILE_LIMIT + 1),
    );
    finding(&fixture.json(&["--static"]).1, "file.read_error");
    fixture.write("package.json", "{}");
    fs::set_permissions(
        fixture.project.path().join("package.json"),
        fs::Permissions::from_mode(0o0),
    )
    .unwrap();
    if fs::read(fixture.project.path().join("package.json")).is_err() {
        finding(&fixture.json(&["--static"]).1, "file.read_error");
    }
    fs::set_permissions(
        fixture.project.path().join("package.json"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let mut reader =
        repo_doctor::safety::FileReader::new(fixture.project.path().canonicalize().unwrap());
    fixture.write("large", &"x".repeat(repo_doctor::safety::FILE_LIMIT));
    for _ in 0..16 {
        assert!(reader.read("large").await.is_ok());
    }
    assert!(reader.read("large").await.is_err());
    fixture.write(".env.example", "SAFE_NAME=example\n");
    fixture.write(
        ".repo-doctor.toml",
        "schema=1\n[env]\nfiles=['local ü file.env']\n",
    );
    fixture.write("local ü file.env", "SAFE_NAME=synthetic\n");
    assert_eq!(fixture.json(&["--static"]).1["summary"]["error"], 0);
}

#[tokio::test]
async fn probes_are_bounded_and_reap_children() {
    let host = tempfile::tempdir().unwrap();
    let executable_path = host.path().join("runtime");
    for (script, expected) in [
        ("exit 9", "nonzero"),
        ("printf '\\377'", "UTF-8"),
        ("exec /bin/sleep 5", "deadline"),
        (
            "while :; do printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'; done",
            "16 KiB",
        ),
        (
            "while :; do printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx' >&2; done",
            "16 KiB",
        ),
    ] {
        executable(&executable_path, script);
        let start = Instant::now();
        let budget = if expected == "deadline" {
            Duration::from_millis(150)
        } else {
            Duration::from_secs(3)
        };
        let error = repo_doctor::runner::probe(&executable_path, &["--version"], budget)
            .await
            .unwrap_err();
        assert!(error.contains(expected), "{script}: {error}");
        let maximum = if expected == "deadline" {
            Duration::from_secs(2)
        } else {
            Duration::from_secs(4)
        };
        assert!(start.elapsed() < maximum);
    }
    let pid_file = host.path().join("pid");
    executable(
        &executable_path,
        &format!(
            "printf '%s' \"$$\" > '{}'\nexec /bin/sleep 5",
            pid_file.display()
        ),
    );
    let start = Instant::now();
    let error =
        repo_doctor::runner::probe(&executable_path, &["--version"], Duration::from_secs(3))
            .await
            .unwrap_err();
    assert!(error.contains("deadline"));
    assert!(start.elapsed() < Duration::from_secs(4));
    let pid: i32 = fs::read_to_string(pid_file).unwrap().parse().unwrap();
    assert_eq!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None),
        Err(nix::errno::Errno::ESRCH)
    );
}

#[test]
fn missing_nonzero_malformed_and_python_fallback() {
    let fixture = Fixture::new();
    fixture.write(".nvmrc", "22");
    finding(&fixture.json(&[]).1, "runtime.missing");
    for script in ["exit 1", "printf 'not a version\\n'"] {
        fixture.executable("node", script);
        let (_, report) = fixture.json(&[]);
        finding(&report, "runtime.probe_error");
        finding(&report, "runtime.prerequisite_unverified");
    }
    fs::remove_file(fixture.project.path().join(".nvmrc")).unwrap();
    fixture.write(".python-version", "3.12");
    fixture.executable("python", "printf 'Python 3.12.2\\n'");
    assert_eq!(fixture.json(&[]).1["outcome"], "passed");
    fixture.executable("python3", "printf 'Python 2.7.18\\n'");
    assert_eq!(fixture.json(&[]).1["outcome"], "passed");
    fs::remove_file(fixture.host.path().join("python")).unwrap();
    finding(&fixture.json(&[]).1, "runtime.probe_error");
}

#[test]
fn cancellation_produces_a_report_and_reaps_the_probe() {
    for signal in [
        nix::sys::signal::Signal::SIGINT,
        nix::sys::signal::Signal::SIGTERM,
    ] {
        assert_cancellation_cleanup(signal);
    }
}

fn assert_cancellation_cleanup(signal: nix::sys::signal::Signal) {
    let fixture = Fixture::new();
    fixture.write(".nvmrc", "22");
    let pid_file = fixture.host.path().join("child-pid");
    fixture.executable(
        "node",
        &format!(
            "printf '%s' \"$$\" > '{}'\nexec /bin/sleep 10",
            pid_file.display()
        ),
    );
    let mut child = fixture
        .command()
        .args(["--format", "json"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !pid_file.exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(child.id() as i32), signal).unwrap();
    while child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    let report = serde_json::from_slice(&output.stdout).unwrap();
    common::validate(&report);
    finding(&report, "execution.incomplete");
    assert_eq!(output.status.code(), Some(1));
    let pid: i32 = fs::read_to_string(pid_file).unwrap().parse().unwrap();
    assert_eq!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None),
        Err(nix::errno::Errno::ESRCH)
    );
}

#[tokio::test]
async fn expired_run_deadline_preserves_incomplete_checks() {
    let fixture = Fixture::new();
    fixture.executable("node", "printf 'v22.1.0\\n'");
    let mut host = repo_doctor::runner::Host {
        root: fixture.project.path().into(),
        path: Some(fixture.host.path().into()),
        deadline: Instant::now(),
        cancelled: false,
        filesystem_cancelled: Default::default(),
    };
    assert!(matches!(
        host.runtime("node").await,
        repo_doctor::runner::Observation::Unverified(_)
    ));
    let endpoint = repo_doctor::config::Endpoint {
        name: "api".into(),
        address: "127.0.0.1".parse().unwrap(),
        port: 1234,
    };
    let check = repo_doctor::checks::sockets::check(&endpoint, false, true, Instant::now()).await;
    assert_eq!(check.status, repo_doctor::model::Status::Skipped);
    assert!(!check.evaluated);
}

#[tokio::test]
async fn blocked_filesystem_work_does_not_hold_the_deadline() {
    use repo_doctor::safety::{FileReader, within_deadline};
    use std::sync::mpsc;

    let (release, wait) = mpsc::channel();
    let (finished, completion) = mpsc::channel();
    let start = Instant::now();
    let cancelled = repo_doctor::safety::Cancellation::default();
    let result = within_deadline(start + Duration::from_millis(100), &cancelled, move || {
        wait.recv().unwrap();
        finished.send(()).unwrap();
    })
    .await;
    assert_eq!(result, Err("The whole-run deadline was exceeded."));
    assert!(start.elapsed() < Duration::from_secs(1));
    release.send(()).unwrap();
    completion.recv_timeout(Duration::from_secs(1)).unwrap();

    let fixture = Fixture::new();
    fixture.write("package.json", "{}");
    let mut reader = FileReader::new(fixture.project.path().canonicalize().unwrap());
    reader.deadline = Instant::now();
    for result in [
        reader.read("package.json").await.map(|_| ()),
        reader.present("package.json").await.map(|_| ()),
        reader.boundary("package.json").await,
    ] {
        assert_eq!(result, Err("The whole-run deadline was exceeded."));
    }
    let (sent, received) = mpsc::channel();
    assert!(
        within_deadline(Instant::now(), &cancelled, move || sent.send(()))
            .await
            .is_err()
    );
    assert_eq!(received.try_recv(), Err(mpsc::TryRecvError::Disconnected));
    cancelled.cancel();
    assert_eq!(
        within_deadline(Instant::now() + Duration::from_secs(1), &cancelled, || ()).await,
        Err("The run was cancelled.")
    );
}

#[cfg(target_os = "linux")]
#[test]
fn stalled_file_read_returns_collected_findings_on_deadline_and_cancellation() {
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;
    use std::process::{Command, Stdio};

    let fixture = Fixture::new();
    let library = fixture.host.path().join("stall-file.so");
    let compile = Command::new("cc")
        .args(["-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"])
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/helpers/stall_file.c"
        ))
        .args(["-ldl", "-o"])
        .arg(&library)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    fixture.write(".nvmrc", "22");
    fixture.write(".env.example", "SAFE_NAME=synthetic-template\n");
    fixture.write(".env.local", "SAFE_NAME=synthetic-value\n");
    fixture.write(
        "package.json",
        r#"{"engines":{"node":">=22"},"dependencies":{"synthetic":"1"}}"#,
    );
    fs::create_dir_all(fixture.project.path().join("node_modules/synthetic")).unwrap();
    fixture.write("node_modules/synthetic/package.json", "{}");
    fixture.write("node_modules/.modules.yaml", "included:\n  dependencies: true\n  devDependencies: true\n  optionalDependencies: true\nlayoutVersion: 5\npackageManager: pnpm@10.13.1\npendingBuilds: []\n");
    let marker = fixture.host.path().join("started");

    for file in [
        ".env.local",
        "node_modules/.modules.yaml",
        "node_modules/synthetic/package.json",
    ] {
        for signal in [None, Some(Signal::SIGINT), Some(Signal::SIGTERM)] {
            let _ = fs::remove_file(&marker);
            let start = Instant::now();
            let mut child = fixture
                .command()
                .args(["--format", "json"])
                .env("LD_PRELOAD", &library)
                .env(
                    "REPO_DOCTOR_TEST_STALL_FILE",
                    fixture.project.path().canonicalize().unwrap().join(file),
                )
                .env("REPO_DOCTOR_TEST_STALL_MARKER", &marker)
                .env("SAFE_NAME", "DO_NOT_PRINT_STALLED_FILE_SECRET")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            while !marker.exists() {
                if start.elapsed() > Duration::from_secs(5) {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("The controlled file read did not start.");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            if let Some(signal) = signal {
                kill(Pid::from_raw(child.id() as i32), signal).unwrap();
            }
            let limit = if signal.is_some() { 5 } else { 18 };
            while child.try_wait().unwrap().is_none() {
                if start.elapsed() > Duration::from_secs(limit) {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("A stalled file read held the command open.");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let output = child.wait_with_output().unwrap();
            assert_eq!(output.status.code(), Some(1));
            assert!(output.stderr.is_empty());
            let report = serde_json::from_slice(&output.stdout).unwrap();
            common::validate(&report);
            finding(&report, "execution.incomplete");
            finding(&report, "env.present");
            finding(&report, "runtime.unverified");
            assert_eq!(report["complete"], false);
            assert_eq!(report["project"]["runtimes"][0], "node");
            assert!(
                !String::from_utf8_lossy(&output.stdout)
                    .contains("DO_NOT_PRINT_STALLED_FILE_SECRET")
            );
            if signal.is_none() {
                assert!(start.elapsed() >= Duration::from_secs(14));
            }
        }
    }
}
