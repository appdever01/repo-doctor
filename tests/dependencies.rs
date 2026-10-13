mod common;
use common::{Fixture, finding};
use std::fs;
use std::os::unix::fs::symlink;

fn fixture() -> Fixture {
    let fixture = Fixture::new();
    fixture.executable("node", "printf 'v22.1.0\\n'");
    fixture.executable("pnpm", "touch forbidden");
    fixture
}

fn install(fixture: &Fixture, name: &str) {
    let directory = fixture.project.path().join(format!("node_modules/{name}"));
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("package.json"),
        r#"{"name":"synthetic","main":"index.js","scripts":{"install":"touch forbidden"}}"#,
    )
    .unwrap();
    fs::write(
        directory.join("index.js"),
        "throw new Error('must not execute');",
    )
    .unwrap();
}

fn metadata(fixture: &Fixture, extra: &str, dev: bool) {
    fs::create_dir_all(fixture.project.path().join("node_modules")).unwrap();
    fixture.write("node_modules/.modules.yaml", &format!("included:\n  dependencies: true\n  devDependencies: {dev}\n  optionalDependencies: true\nlayoutVersion: 5\nnodeLinker: isolated\npackageManager: pnpm@10.13.1\n{extra}"));
}

#[test]
fn missing_dependencies_corrected_without_loading_code_or_running_scripts() {
    let fixture = fixture();
    fixture.write("package.json", r#"{"engines":{"node":">=22"},"dependencies":{"is-number":"7.0.0","optional":"1"},"devDependencies":{"@scope/tool":"npm:real@1"},"optionalDependencies":{"optional":"1"},"peerDependencies":{"consumer":"1"},"scripts":{"start":"touch forbidden","postinstall":"touch forbidden"}}"#);
    let (_, report) = fixture.json(&[]);
    assert_eq!(report["exit_code"], 1);
    assert_eq!(report["summary"]["fail"], 2);
    finding(&report, "dependencies.package_missing");
    install(&fixture, "is-number");
    install(&fixture, "@scope/tool");
    let before = fs::read(fixture.project.path().join("package.json")).unwrap();
    let (_, report) = fixture.json(&["--strict"]);
    assert_eq!(report["outcome"], "passed");
    assert_eq!(report["summary"]["pass"], 4);
    assert_eq!(
        before,
        fs::read(fixture.project.path().join("package.json")).unwrap()
    );
    assert!(!fixture.project.path().join("forbidden").exists());
}

#[test]
fn pnpm_ignored_and_pending_builds_fail_including_static_mode() {
    let fixture = fixture();
    fixture.write("package.json", r#"{"engines":{"node":">=22"},"packageManager":"pnpm@10.13.1","dependencies":{"better-sqlite3":"12.4.1"}}"#);
    install(&fixture, "better-sqlite3");
    metadata(
        &fixture,
        "ignoredBuilds:\n  - better-sqlite3\npendingBuilds:\n  - '@scope/native@1.2.3(peer@4)'\nregistries:\n  default: https://secret:DO_NOT_PRINT@invalid\n",
        true,
    );
    let before = fs::read(fixture.project.path().join("node_modules/.modules.yaml")).unwrap();
    for flags in [vec![], vec!["--strict"], vec!["--static"]] {
        let (output, report) = fixture.json(&flags);
        assert_eq!(report["exit_code"], 1);
        let blocked = finding(&report, "dependencies.pnpm_builds_blocked");
        assert!(
            blocked["message"]
                .as_str()
                .unwrap()
                .contains("1 blocked and 1 pending")
        );
        assert!(!String::from_utf8_lossy(&output.stdout).contains("DO_NOT_PRINT"));
    }
    assert_eq!(
        before,
        fs::read(fixture.project.path().join("node_modules/.modules.yaml")).unwrap()
    );
    assert!(!fixture.project.path().join("forbidden").exists());
    metadata(&fixture, "ignoredBuilds: []\npendingBuilds: []\n", true);
    assert_eq!(fixture.json(&["--strict"]).1["outcome"], "passed");
}

#[test]
fn absent_pnpm_metadata_and_production_omissions_remain_unverified() {
    let fixture = fixture();
    fixture.write("package.json", r#"{"engines":{"node":">=22"},"packageManager":"pnpm@10.13.1","dependencies":{"runtime":"1"},"devDependencies":{"dev":"1"}}"#);
    install(&fixture, "runtime");
    install(&fixture, "dev");
    let (_, report) = fixture.json(&[]);
    finding(&report, "dependencies.pnpm_metadata_unverified");
    assert_eq!(report["complete"], false);
    assert_eq!(fixture.json(&["--strict"]).1["exit_code"], 1);
    fs::remove_dir_all(fixture.project.path().join("node_modules/dev")).unwrap();
    metadata(&fixture, "pendingBuilds: []\n", false);
    let (_, report) = fixture.json(&[]);
    finding(&report, "dependencies.dev_omitted");
    assert_eq!(report["summary"]["fail"], 0);
    assert_eq!(report["complete"], false);
}

#[test]
fn unsupported_metadata_is_not_a_passing_build_check_and_omits_raw_fields() {
    let fixture = fixture();
    fixture.write("package.json", "{}");
    for extra in [
        "pendingBuilds: &DO_NOT_PRINT []\n",
        "pendingBuilds: []\npendingBuilds: []\n",
        "pendingBuilds:\n",
        "pendingBuilds: []\n  - hidden\n",
        "ignoredBuilds:\n  - *DO_NOT_PRINT\npendingBuilds: []\n",
        "pendingBuilds: [DO_NOT_PRINT]\n",
    ] {
        metadata(&fixture, extra, true);
        let (output, report) = fixture.json(&[]);
        finding(&report, "dependencies.pnpm_metadata_unverified");
        assert_eq!(report["complete"], false);
        assert_eq!(report["exit_code"], 1);
        assert!(!String::from_utf8_lossy(&output.stdout).contains("DO_NOT_PRINT"));
    }
    metadata(&fixture, "pendingBuilds: []\n", true);
    let path = fixture.project.path().join("node_modules/.modules.yaml");
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("layoutVersion: 5", "layoutVersion: 99");
    fs::write(path, text).unwrap();
    finding(
        &fixture.json(&[]).1,
        "dependencies.pnpm_metadata_unverified",
    );
}

#[test]
fn dependency_and_build_record_limits_are_enforced() {
    let fixture = fixture();
    let packages: serde_json::Map<String, serde_json::Value> = (0..257)
        .map(|i| (format!("pkg-{i}"), serde_json::json!("1")))
        .collect();
    fixture.write(
        "package.json",
        &serde_json::json!({"dependencies":packages}).to_string(),
    );
    finding(&fixture.json(&[]).1, "dependencies.declaration_invalid");
    fixture.write(
        "package.json",
        r#"{"dependencies":{"valid":"DO_NOT_PRINT"}}"#,
    );
    let records = (0..257)
        .map(|i| format!("  - pkg-{i}\n"))
        .collect::<String>();
    metadata(
        &fixture,
        &format!("ignoredBuilds:\n{records}pendingBuilds: []\n"),
        true,
    );
    let (output, report) = fixture.json(&[]);
    finding(&report, "dependencies.pnpm_metadata_unverified");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("DO_NOT_PRINT"));
    fixture.write(
        "node_modules/.modules.yaml",
        &"x".repeat(repo_doctor::safety::FILE_LIMIT + 1),
    );
    finding(
        &fixture.json(&[]).1,
        "dependencies.pnpm_metadata_unverified",
    );
}

#[test]
fn dependency_paths_stay_inside_the_selected_directory() {
    let fixture = fixture();
    fixture.write(
        "package.json",
        r#"{"engines":{"node":">=22"},"dependencies":{"linked":"1"}}"#,
    );
    install(&fixture, "actual");
    symlink("actual", fixture.project.path().join("node_modules/linked")).unwrap();
    assert_eq!(fixture.json(&[]).1["outcome"], "passed");
    fs::remove_file(fixture.project.path().join("node_modules/linked")).unwrap();
    symlink(
        fixture.host.path(),
        fixture.project.path().join("node_modules/linked"),
    )
    .unwrap();
    finding(&fixture.json(&[]).1, "dependencies.package_unverified");
    fs::remove_file(fixture.project.path().join("node_modules/linked")).unwrap();
    symlink(
        "missing",
        fixture.project.path().join("node_modules/linked"),
    )
    .unwrap();
    finding(&fixture.json(&[]).1, "dependencies.package_unverified");
    for name in [
        "../DO_NOT_PRINT",
        "@scope/../../DO_NOT_PRINT",
        "bad\\path",
        ".pnpm",
        "control\nname",
    ] {
        fixture.write(
            "package.json",
            &serde_json::json!({"dependencies":{name:"1"}}).to_string(),
        );
        let (output, report) = fixture.json(&[]);
        finding(&report, "dependencies.declaration_invalid");
        assert!(!String::from_utf8_lossy(&output.stdout).contains("DO_NOT_PRINT"));
    }
}

#[test]
fn installed_manifest_errors_and_pnp_loaders_stay_unverified() {
    let fixture = fixture();
    fixture.write(
        "package.json",
        r#"{"engines":{"node":">=22"},"dependencies":{"valid":"1"}}"#,
    );
    install(&fixture, "valid");
    for text in [
        "{DO_NOT_PRINT".to_owned(),
        "[]".to_owned(),
        "x".repeat(repo_doctor::safety::FILE_LIMIT + 1),
    ] {
        fixture.write("node_modules/valid/package.json", &text);
        let (output, report) = fixture.json(&[]);
        finding(&report, "dependencies.package_unverified");
        assert!(!String::from_utf8_lossy(&output.stdout).contains("DO_NOT_PRINT"));
    }
    fixture.write(".pnp.cjs", "require('fs').writeFileSync('forbidden', 'x');");
    let (_, report) = fixture.json(&[]);
    finding(&report, "dependencies.layout_unverified");
    assert_eq!(report["summary"]["fail"], 0);
    assert_eq!(report["complete"], false);
    assert!(!fixture.project.path().join("forbidden").exists());
}

#[tokio::test]
async fn exhausted_combined_budget_stops_additional_file_inspection() {
    let fixture = fixture();
    fixture.write("large", &"x".repeat(repo_doctor::safety::FILE_LIMIT));
    fs::create_dir(fixture.project.path().join("not-a-file")).unwrap();
    let mut reader =
        repo_doctor::safety::FileReader::new(fixture.project.path().canonicalize().unwrap());
    for _ in 0..16 {
        reader.read("large").await.unwrap();
    }
    assert_eq!(
        reader.read("not-a-file").await.unwrap_err(),
        "Inputs exceed the combined 16 MiB limit."
    );
}
