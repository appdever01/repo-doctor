use crate::checks::dependencies::DependencyPlan;
use crate::checks::runtime::Runtime;
use crate::config::Config;
use crate::model::{CheckResult, Status};
use crate::runner::{Host, Selection};
use crate::safety::FileReader;
use std::collections::BTreeSet;

pub struct ProjectRequirements {
    pub runtimes: Vec<Runtime>,
    pub findings: Vec<CheckResult>,
    pub supported: bool,
    pub dependencies: Option<DependencyPlan>,
}

async fn read(
    reader: &mut FileReader,
    name: &str,
    findings: &mut Vec<CheckResult>,
) -> Option<String> {
    match reader.read(name).await {
        Ok(input) => input,
        Err(reason) => {
            findings.push(CheckResult::new(
                &format!("file.{name}"),
                Status::Error,
                "file.read_error",
                reason,
                name,
            ));
            None
        }
    }
}

pub async fn discover(
    reader: &mut FileReader,
    config: &Config,
    configured: bool,
    host: &Host,
    static_mode: bool,
) -> ProjectRequirements {
    let mut findings = vec![];
    let mut node = Runtime::new("node");
    let mut python = Runtime::new("python");
    let mut node_present = config.runtimes.node.is_some();
    let mut python_present = config.runtimes.python.is_some();
    let mut managers = BTreeSet::new();
    let mut package_found = false;
    let mut dependencies = None;
    if let Some(input) = read(reader, "package.json", &mut findings).await {
        node_present = true;
        package_found = true;
        match serde_json::from_str::<serde_json::Value>(&input) {
            Ok(value) if value.is_object() => {
                match DependencyPlan::parse(&value) {
                    Ok(plan) => dependencies = Some(plan),
                    Err(reason) => findings.push(CheckResult::new(
                        "dependencies.declarations",
                        Status::Error,
                        "dependencies.declaration_invalid",
                        reason,
                        "package.json",
                    )),
                }
                if let Some(engines) = value.get("engines") {
                    if !engines.is_object() {
                        findings.push(CheckResult::new(
                            "file.package.engines",
                            Status::Error,
                            "manifest.field_error",
                            "engines must be an object.",
                            "package.json",
                        ));
                    }
                    if let Some(range) = engines.get("node") {
                        match range.as_str() {
                            Some(range) => node.add(range, "package.json", false),
                            None => node.unsupported.push("package.json".into()),
                        }
                    }
                }
                if let Some(manager) = value.get("packageManager") {
                    if let Some(name) = manager
                        .as_str()
                        .and_then(|s| s.split('@').next())
                        .filter(|n| ["npm", "pnpm", "yarn", "bun"].contains(n))
                    {
                        managers.insert(name.to_owned());
                    } else {
                        findings.push(
                            CheckResult::new(
                                "package_manager.declaration",
                                Status::Warn,
                                "package_manager.unsupported",
                                "The package-manager declaration is unsupported.",
                                "package.json",
                            )
                            .incomplete(),
                        );
                    }
                }
            }
            _ => findings.push(CheckResult::new(
                "file.package",
                Status::Error,
                "manifest.parse_error",
                "package.json is not a valid JSON object.",
                "package.json",
            )),
        }
    }
    if let Some(input) = read(reader, "pyproject.toml", &mut findings).await {
        python_present = true;
        match toml::from_str::<toml::Value>(&input) {
            Ok(value) => {
                if let Some(project) = value.get("project") {
                    if !project.is_table() {
                        findings.push(CheckResult::new(
                            "file.pyproject.project",
                            Status::Error,
                            "manifest.field_error",
                            "project must be a TOML table.",
                            "pyproject.toml",
                        ));
                    }
                    if let Some(range) = project.get("requires-python") {
                        match range.as_str() {
                            Some(range) => python.add(range, "pyproject.toml", false),
                            None => python.unsupported.push("pyproject.toml".into()),
                        }
                    }
                }
            }
            Err(_) => findings.push(CheckResult::new(
                "file.pyproject",
                Status::Error,
                "manifest.parse_error",
                "pyproject.toml contains invalid TOML.",
                "pyproject.toml",
            )),
        }
    }
    for name in [".nvmrc", ".node-version"] {
        if let Some(value) = read(reader, name, &mut findings).await {
            node_present = true;
            node.add(&value, name, true);
        }
    }
    if let Some(value) = read(reader, ".python-version", &mut findings).await {
        python_present = true;
        python.add(&value, ".python-version", true);
    }
    if let Some(value) = &config.runtimes.node {
        node.add(value, ".repo-doctor.toml", false);
    }
    if let Some(value) = &config.runtimes.python {
        python.add(value, ".repo-doctor.toml", false);
    }
    for (name, manager) in [
        ("package-lock.json", "npm"),
        ("pnpm-lock.yaml", "pnpm"),
        ("yarn.lock", "yarn"),
        ("bun.lock", "bun"),
        ("bun.lockb", "bun"),
    ] {
        match reader.present(name).await {
            Ok(true) => {
                managers.insert(manager.into());
            }
            Ok(false) => (),
            Err(reason) => findings.push(CheckResult::new(
                &format!("file.{name}"),
                Status::Error,
                "file.read_error",
                reason,
                name,
            )),
        }
    }
    if managers.len() > 1 {
        findings.push(
            CheckResult::new(
                "package_manager.conflict",
                Status::Warn,
                "package_manager.conflict",
                "Package-manager declarations or lockfile hints disagree.",
                "root lockfiles and package.json",
            )
            .incomplete()
            .next("Review the package-manager declaration and keep the intended lockfile."),
        );
    }
    if let Some(plan) = &mut dependencies {
        plan.pnpm = managers.contains("pnpm");
    }
    let bun_only = managers.len() == 1
        && managers.contains("bun")
        && node.requirements.is_empty()
        && node.unsupported.is_empty();
    for manager in managers {
        let id = format!("package_manager.{manager}");
        if manager == "bun" {
            findings.push(
                CheckResult::new(
                    &id,
                    Status::Warn,
                    "package_manager.bun_unsupported",
                    "Bun runtime checks are unsupported; Bun is not assumed to use Node.js.",
                    "root lockfiles and package.json",
                )
                .incomplete(),
            );
            continue;
        }
        if static_mode {
            findings.push(CheckResult::new(
                &id,
                Status::Skipped,
                "package_manager.launcher_unverified",
                "Static mode does not inspect host launchers.",
                "root lockfiles and package.json",
            ));
            continue;
        }
        findings.push(match host.select(&manager).await {
            Selection::Trusted(_) => CheckResult::new(
                &id,
                Status::Pass,
                "package_manager.launcher_present",
                format!("{manager} launcher is present; it was not executed."),
                "root lockfiles and package.json",
            ),
            Selection::Missing => CheckResult::new(
                &id,
                Status::Fail,
                "package_manager.launcher_missing",
                format!("{manager} launcher is absent from PATH."),
                "root lockfiles and package.json",
            )
            .next(format!(
                "Install or select the {manager} launcher outside Repo Doctor."
            )),
            Selection::Unverified(reason) => CheckResult::new(
                &id,
                Status::Skipped,
                "package_manager.launcher_unverified",
                reason,
                "host PATH",
            ),
        });
    }
    let mut runtimes = vec![];
    if node_present && !bun_only {
        runtimes.push(node);
    }
    if python_present {
        runtimes.push(python);
    }
    ProjectRequirements {
        supported: configured || package_found || node_present || python_present,
        runtimes,
        findings,
        dependencies,
    }
}
