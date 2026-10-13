mod common;
use common::{Fixture, finding};
use std::process::Command;

#[test]
fn help_version_and_syntax() {
    for flag in ["--help", "--version"] {
        assert!(
            Command::new(env!("CARGO_BIN_EXE_repo-doctor"))
                .arg(flag)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    for args in [
        vec!["check", "--unknown"],
        vec!["check", "--format", "yaml"],
        vec!["unknown"],
    ] {
        assert_eq!(
            Command::new(env!("CARGO_BIN_EXE_repo-doctor"))
                .args(args)
                .output()
                .unwrap()
                .status
                .code(),
            Some(2)
        );
    }
}

#[test]
fn unsupported_input_errors_and_no_parent_search() {
    let fixture = Fixture::new();
    let (_, report) = fixture.json(&[]);
    assert_eq!(report["outcome"], "incomplete");
    assert_eq!(report["exit_code"], 0);
    assert_eq!(fixture.json(&["--strict"]).1["exit_code"], 1);
    let output = Command::new(env!("CARGO_BIN_EXE_repo-doctor"))
        .args([
            "check",
            "/repo-doctor-nonexistent-target",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    let report = serde_json::from_slice(&output.stdout).unwrap();
    common::validate(&report);
    assert_eq!(output.status.code(), Some(2));
    fixture.write("package.json", "{}");
    std::fs::create_dir(fixture.project.path().join("nested")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_repo-doctor"))
        .arg("check")
        .arg(fixture.project.path().join("nested"))
        .args(["--static", "--format", "json"])
        .output()
        .unwrap();
    let report = serde_json::from_slice(&output.stdout).unwrap();
    finding(&report, "project.unsupported");
}

#[test]
fn config_validation_and_json_errors() {
    let fixture = Fixture::new();
    for input in [
        "schema=2",
        "schema=1\nunknown=true",
        "schema=1\n[env]\nfiles=['../escape']",
        "schema=1\n[env]\nrequired=['X']\noptional=['X']",
        "schema=1\n[env]\nrequired=['BAD-NAME']",
        "schema=1\n[[services]]\nname='db'\naddress='example.com'\nport=5432",
        "schema=1\n[[ports]]\nname='api'\naddress='192.168.1.1'\nport=3000",
        "schema=1\n[[ports]]\nname='API'\naddress='127.0.0.1'\nport=0",
        "schema=1\n[runtimes]\nnode='lts/*'",
        "schema=1\n[env]\ntemplate='/tmp/example'",
    ] {
        fixture.write(".repo-doctor.toml", input);
        let (_, report) = fixture.json(&[]);
        assert_eq!(report["outcome"], "invalid_input");
        assert_eq!(report["exit_code"], 2);
    }
    let endpoint = "\n[[ports]]\nname='api'\naddress='127.0.0.1'\nport=65536";
    fixture.write(".repo-doctor.toml", &format!("schema=1{endpoint}"));
    assert_eq!(fixture.json(&[]).1["exit_code"], 2);
    fixture.write(".repo-doctor.toml", "schema=1\n[[ports]]\nname='api'\naddress='127.0.0.1'\nport=3000\n[[services]]\nname='api'\naddress='::1'\nport=3001");
    assert_eq!(fixture.json(&[]).1["exit_code"], 2);
}

#[test]
fn malformed_manifests_continue_environment_checks_and_no_requirement_is_incomplete() {
    let fixture = Fixture::new();
    fixture.executable("node", "printf 'v22.1.0\\n'");
    fixture.write("package.json", "{bad}");
    fixture.write("pyproject.toml", "[project");
    fixture.write(".env.example", "REPO_DOCTOR_TEST_MISSING=value\n");
    let (_, report) = fixture.json(&["--static"]);
    finding(&report, "manifest.parse_error");
    finding(&report, "env.missing");
    fixture.write("package.json", "{}");
    std::fs::remove_file(fixture.project.path().join("pyproject.toml")).unwrap();
    std::fs::remove_file(fixture.project.path().join(".env.example")).unwrap();
    let (_, report) = fixture.json(&[]);
    assert_eq!(report["outcome"], "incomplete");
    finding(&report, "runtime.requirement_missing");
}

#[test]
fn every_runtime_declaration_is_evaluated_and_aliases_stay_unverified() {
    let fixture = Fixture::new();
    fixture.executable("node", "printf 'v22.1.0\\n'");
    fixture.write("package.json", r#"{"engines":{"node":">=24"}}"#);
    fixture.write(
        ".repo-doctor.toml",
        "schema=1\n[runtimes]\nnode='>=22 <23'\n",
    );
    fixture.write(".node-version", "22.1");
    fixture.write(".nvmrc", "lts/*");
    let (_, report) = fixture.json(&[]);
    finding(&report, "runtime.version_matches");
    finding(&report, "runtime.version_mismatch");
    finding(&report, "runtime.unsupported_requirement");
    assert_eq!(report["complete"], false);
}

#[test]
fn package_manager_presence_conflict_and_bun() {
    let fixture = Fixture::new();
    fixture.executable("pnpm", "touch should-never-run");
    fixture.write("package.json", r#"{"packageManager":"pnpm@10.0.0"}"#);
    fixture.write("package-lock.json", "{}");
    let (_, report) = fixture.json(&[]);
    finding(&report, "package_manager.launcher_present");
    finding(&report, "package_manager.conflict");
    finding(&report, "package_manager.launcher_missing");
    assert!(!fixture.project.path().join("should-never-run").exists());
    fixture.write("package.json", r#"{"packageManager":"bun@1.0.0"}"#);
    std::fs::remove_file(fixture.project.path().join("package-lock.json")).unwrap();
    fixture.write("bun.lockb", "\u{0}");
    let (_, report) = fixture.json(&[]);
    finding(&report, "package_manager.bun_unsupported");
    assert_eq!(report["project"]["runtimes"], serde_json::json!([]));
}
