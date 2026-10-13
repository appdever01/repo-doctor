use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

pub struct Fixture {
    pub project: TempDir,
    pub host: TempDir,
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}

impl Fixture {
    pub fn new() -> Self {
        Self {
            project: tempfile::tempdir().unwrap(),
            host: tempfile::tempdir().unwrap(),
        }
    }
    pub fn write(&self, name: &str, text: &str) {
        fs::write(self.project.path().join(name), text).unwrap();
    }
    pub fn executable(&self, name: &str, script: &str) {
        executable(&self.host.path().join(name), script);
    }
    pub fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_repo-doctor"));
        command
            .env_clear()
            .env("PATH", self.host.path())
            .arg("check")
            .arg(self.project.path());
        command
    }
    pub fn json(&self, flags: &[&str]) -> (Output, Value) {
        let output = self
            .command()
            .args(["--format", "json"])
            .args(flags)
            .output()
            .unwrap();
        let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
            panic!("Invalid JSON: {}", String::from_utf8_lossy(&output.stdout))
        });
        assert!(output.stderr.is_empty());
        assert_eq!(
            output.status.code().unwrap(),
            value["exit_code"].as_i64().unwrap() as i32
        );
        validate(&value);
        (output, value)
    }
}

pub fn executable(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, format!("#!/bin/sh\n{script}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

pub fn validate(value: &Value) {
    let schema: Value =
        serde_json::from_str(include_str!("../../schemas/report-v1.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    assert!(validator.is_valid(value), "Schema violation: {value}");
    let checks = value["checks"].as_array().unwrap();
    for status in ["pass", "warn", "fail", "skipped", "error"] {
        assert_eq!(
            value["summary"][status].as_u64().unwrap() as usize,
            checks.iter().filter(|c| c["status"] == status).count()
        );
    }
    let ids: Vec<_> = checks.iter().map(|c| c["id"].as_str().unwrap()).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(ids, sorted);
}

pub fn finding<'a>(value: &'a Value, code: &str) -> &'a Value {
    value["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["finding_code"] == code)
        .unwrap_or_else(|| panic!("Missing {code}: {value}"))
}
