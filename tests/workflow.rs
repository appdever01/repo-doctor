mod common;
use common::{Fixture, finding};
use std::fs;
use std::net::TcpListener;

#[test]
fn node_broken_to_corrected_and_static_workflow() {
    let fixture = Fixture::new();
    fixture.executable("node", "printf 'v22.1.0\\n'");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    fixture.write(
        "package.json",
        r#"{"engines":{"node":">=24 <25"},"scripts":{"preinstall":"touch forbidden"}}"#,
    );
    fixture.write(
        ".env.example",
        "REPO_DOCTOR_TEST_SECRET=synthetic-template-only\n",
    );
    fixture.write(
        ".repo-doctor.toml",
        &format!("schema=1\n[[ports]]\nname='api'\naddress='127.0.0.1'\nport={port}\n"),
    );
    let (_, report) = fixture.json(&[]);
    assert_eq!(report["exit_code"], 1);
    finding(&report, "runtime.version_mismatch");
    finding(&report, "env.missing");
    finding(&report, "port.occupied");
    let (_, static_report) = fixture.json(&["--static"]);
    finding(&static_report, "runtime.unverified");
    finding(&static_report, "socket.skipped");
    assert_eq!(
        fs::read_to_string(fixture.project.path().join(".env.example")).unwrap(),
        "REPO_DOCTOR_TEST_SECRET=synthetic-template-only\n"
    );
    fixture.write("package.json", r#"{"engines":{"node":">=22 <23"}}"#);
    fixture.write(".env", "REPO_DOCTOR_TEST_SECRET=synthetic-actual\n");
    drop(listener);
    let before = fs::read(fixture.project.path().join("package.json")).unwrap();
    let (_, report) = fixture.json(&[]);
    assert_eq!(report["outcome"], "passed");
    assert_eq!(report["complete"], true);
    assert_eq!(report["exit_code"], 0);
    assert_eq!(
        before,
        fs::read(fixture.project.path().join("package.json")).unwrap()
    );
    assert!(!fixture.project.path().join("forbidden").exists());
    let human = fixture.command().output().unwrap();
    assert_eq!(human.status.code(), Some(0));
    let human = String::from_utf8(human.stdout).unwrap();
    assert!(human.contains("Result: declared setup checks passed; coverage complete"));
    assert!(human.contains("4 passed, 0 warnings, 0 failed, 0 skipped, 0 errors"));
    let (_, report) = fixture.json(&["--static"]);
    assert_eq!(report["outcome"], "incomplete");
    let (_, report) = fixture.json(&["--static", "--strict"]);
    assert_eq!(report["exit_code"], 1);
    assert!(TcpListener::bind(("127.0.0.1", port)).is_ok());
}

#[test]
fn python_broken_to_corrected_then_hybrid() {
    let fixture = Fixture::new();
    fixture.executable("python3", "printf 'Python 3.12.2\\n'");
    fixture.write(
        "pyproject.toml",
        "[project]\nrequires-python='>=3.13,<3.14'\n",
    );
    let (_, report) = fixture.json(&[]);
    finding(&report, "runtime.version_mismatch");
    assert_eq!(report["project"]["runtimes"], serde_json::json!(["python"]));
    fixture.write(
        "pyproject.toml",
        "[project]\nrequires-python='>=3.11,<3.14'\n",
    );
    fixture.write(".python-version", "3.12\n");
    let (_, report) = fixture.json(&[]);
    assert_eq!(report["outcome"], "passed");
    fixture.executable("node", "printf 'v22.1.0\\n'");
    fixture.write(".nvmrc", "22\n");
    let (_, report) = fixture.json(&[]);
    assert_eq!(
        report["project"]["runtimes"],
        serde_json::json!(["node", "python"])
    );
    assert_eq!(report["outcome"], "passed");
}

#[test]
fn services_are_optional_tcp_checks_and_ports_release() {
    let fixture = Fixture::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    fixture.write(
        ".repo-doctor.toml",
        &format!(
            "schema=1\n[[services]]\nname='synthetic-listener'\naddress='127.0.0.1'\nport={port}\n"
        ),
    );
    let (_, report) = fixture.json(&[]);
    assert_eq!(report["outcome"], "incomplete");
    let (_, report) = fixture.json(&["--services"]);
    assert_eq!(report["outcome"], "passed");
    assert!(
        finding(&report, "service.tcp_accepted")["message"]
            .as_str()
            .unwrap()
            .contains("TCP connection accepted")
    );
    drop(listener);
    let (_, report) = fixture.json(&["--services"]);
    assert_eq!(report["exit_code"], 1);
    finding(&report, "service.refused");
    let (_, report) = fixture.json(&["--services", "--static"]);
    assert_eq!(report["outcome"], "incomplete");
}
