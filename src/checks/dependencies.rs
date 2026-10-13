use crate::model::{CheckResult, Status};
use crate::safety::FileReader;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const DEPENDENCY_LIMIT: usize = 256;
const METADATA: &str = "node_modules/.modules.yaml";

pub struct DependencyPlan {
    packages: BTreeMap<String, bool>,
    pub pnpm: bool,
}

fn package_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 214 {
        return false;
    }
    let parts: Vec<_> = name.strip_prefix('@').unwrap_or(name).split('/').collect();
    if parts.len() != if name.starts_with('@') { 2 } else { 1 } {
        return false;
    }
    parts.iter().all(|part| {
        !part.is_empty()
            && !part.starts_with('.')
            && *part != "node_modules"
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    })
}

impl DependencyPlan {
    pub fn parse(value: &Value) -> Result<Self, &'static str> {
        let mut packages = BTreeMap::new();
        let mut optional = BTreeSet::new();
        for field in ["optionalDependencies", "devDependencies", "dependencies"] {
            if let Some(entries) = value.get(field) {
                let entries = entries
                    .as_object()
                    .ok_or("Dependency fields must be objects.")?;
                if entries.len() > DEPENDENCY_LIMIT {
                    return Err("At most 256 direct dependency declarations can be inspected.");
                }
                for (name, spec) in entries {
                    if !package_name(name) || !spec.is_string() {
                        return Err(
                            "Dependency declarations require supported package names and string specifications.",
                        );
                    }
                    if field == "optionalDependencies" {
                        optional.insert(name.clone());
                    } else if !optional.contains(name) {
                        let dev = field == "devDependencies";
                        packages
                            .entry(name.clone())
                            .and_modify(|current| *current &= dev)
                            .or_insert(dev);
                    }
                }
            }
        }
        if packages.len() > DEPENDENCY_LIMIT {
            return Err("At most 256 required direct dependencies can be inspected.");
        }
        Ok(Self {
            packages,
            pnpm: false,
        })
    }
}

#[derive(Default)]
struct Modules {
    ignored: usize,
    pending: usize,
    dev_omitted: bool,
}

fn scalar(raw: &str) -> Result<String, ()> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 512 || raw.chars().any(char::is_control) {
        return Err(());
    }
    if raw.starts_with('"') {
        return serde_json::from_str::<String>(raw).map_err(|_| ());
    }
    if raw.starts_with('\'') {
        return raw
            .strip_prefix('\'')
            .and_then(|s| s.strip_suffix('\''))
            .filter(|s| !s.contains('\''))
            .map(str::to_owned)
            .ok_or(());
    }
    if raw
        .bytes()
        .any(|b| b.is_ascii_whitespace() || b"[]{}#&*!|>,'\"".contains(&b))
    {
        return Err(());
    }
    Ok(raw.to_owned())
}

fn modules(input: &str) -> Result<Modules, &'static str> {
    let mut result = Modules::default();
    let mut seen = BTreeSet::new();
    let mut active = "";
    let mut manager = false;
    let mut layout = false;
    let mut included = false;
    let mut dev_seen = false;
    let mut items = 0;
    let unsupported = "The pnpm installation metadata uses unsupported or malformed fields.";
    for line in input.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if !line.starts_with(char::is_whitespace) {
            if matches!(active, "ignoredBuilds" | "pendingBuilds") && items == 0 {
                return Err(unsupported);
            }
            active = "";
            items = 0;
            let Some((key, raw)) = line.split_once(':') else {
                return Err(unsupported);
            };
            let key = key.trim();
            if key.is_empty() || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                return Err(unsupported);
            }
            if ![
                "packageManager",
                "layoutVersion",
                "ignoredBuilds",
                "pendingBuilds",
                "included",
                "nodeLinker",
            ]
            .contains(&key)
            {
                continue;
            }
            if !seen.insert(key) {
                return Err(unsupported);
            }
            match key {
                "packageManager" => {
                    let value = scalar(raw).map_err(|_| unsupported)?;
                    manager = value
                        .strip_prefix("pnpm@")
                        .is_some_and(|version| node_semver::Version::parse(version).is_ok());
                    if !manager {
                        return Err(unsupported);
                    }
                    active = "closed";
                }
                "layoutVersion" => {
                    layout = raw.trim() == "5";
                    if !layout {
                        return Err(unsupported);
                    }
                    active = "closed";
                }
                "nodeLinker" => {
                    if !matches!(
                        scalar(raw).map_err(|_| unsupported)?.as_str(),
                        "isolated" | "hoisted"
                    ) {
                        return Err(unsupported);
                    }
                    active = "closed";
                }
                "included" => {
                    if !raw.trim().is_empty() {
                        return Err(unsupported);
                    }
                    included = true;
                    active = key;
                }
                _ => {
                    if raw.trim().is_empty() {
                        active = key;
                    } else if raw.trim() == "[]" {
                        active = "closed";
                    } else {
                        return Err(unsupported);
                    }
                }
            }
        } else if active == "included" {
            let (key, raw) = line
                .strip_prefix("  ")
                .and_then(|s| s.split_once(':'))
                .ok_or(unsupported)?;
            if !["dependencies", "devDependencies", "optionalDependencies"].contains(&key)
                || !matches!(raw.trim(), "true" | "false")
            {
                return Err(unsupported);
            }
            if key == "devDependencies" {
                if dev_seen {
                    return Err(unsupported);
                }
                dev_seen = true;
                result.dev_omitted = raw.trim() == "false";
            }
        } else if active == "ignoredBuilds" || active == "pendingBuilds" {
            let value = line.strip_prefix("  - ").ok_or(unsupported)?;
            scalar(value).map_err(|_| unsupported)?;
            items += 1;
            if active == "ignoredBuilds" {
                result.ignored += 1;
            } else {
                result.pending += 1;
            }
            if result.ignored + result.pending > DEPENDENCY_LIMIT {
                return Err("At most 256 pnpm blocked or pending build records can be inspected.");
            }
        } else if active == "closed" {
            return Err(unsupported);
        }
    }
    if !manager
        || !layout
        || !included
        || !dev_seen
        || !seen.contains("pendingBuilds")
        || (matches!(active, "ignoredBuilds" | "pendingBuilds") && items == 0)
    {
        return Err(unsupported);
    }
    Ok(result)
}

fn unverified(id: &str, code: &str, reason: &str, source: &str) -> CheckResult {
    CheckResult::new(id, Status::Error, code, reason, source)
        .next("Review the installation layout or metadata outside Repo Doctor; no dependency scripts were run.")
}

pub async fn check(reader: &mut FileReader, plan: &DependencyPlan) -> Vec<CheckResult> {
    let mut findings = vec![];
    let mut dev_omitted = false;
    match reader.read(METADATA).await {
        Ok(Some(input)) => match modules(&input) {
            Ok(state) => {
                dev_omitted = state.dev_omitted;
                let blocked = state.ignored + state.pending > 0;
                findings.push(CheckResult::new(
                    "dependencies.pnpm-builds",
                    if blocked { Status::Fail } else { Status::Pass },
                    if blocked { "dependencies.pnpm_builds_blocked" } else { "dependencies.pnpm_builds_clear" },
                    if blocked {
                        format!("pnpm records {} blocked and {} pending dependency builds; these may prevent startup.", state.ignored, state.pending)
                    } else {
                        "pnpm records no blocked or pending dependency builds; build outputs were not executed or verified.".into()
                    },
                    METADATA,
                ).next(if blocked {
                    "Review pnpm ignored-builds and the dependency scripts, then decide which builds to trust outside Repo Doctor. No scripts were approved."
                } else {
                    "Verify application startup separately; clear build records do not prove build output integrity."
                }));
            }
            Err(reason) => findings.push(unverified("dependencies.pnpm-builds", "dependencies.pnpm_metadata_unverified", reason, METADATA)),
        },
        Ok(None) if plan.pnpm && !plan.packages.is_empty() => findings.push(
            CheckResult::new("dependencies.pnpm-builds", Status::Skipped, "dependencies.pnpm_metadata_unverified", "pnpm installation metadata is absent; dependency build records are unverified.", METADATA)
                .next("Install dependencies outside Repo Doctor, then check again. No scripts were approved.")),
        Ok(None) => (),
        Err(reason) => findings.push(unverified("dependencies.pnpm-builds", "dependencies.pnpm_metadata_unverified", reason, METADATA)),
    }
    if plan.packages.is_empty() {
        return findings;
    }
    for file in [".pnp.cjs", ".pnp.js"] {
        match reader.present(file).await {
            Ok(true) => {
                findings.push(CheckResult::new(
                    "dependencies.installation",
                    Status::Skipped,
                    "dependencies.layout_unverified",
                    "Plug'n'Play dependency resolution is unsupported; no loader was executed.",
                    file,
                ));
                return findings;
            }
            Err(reason) => {
                findings.push(unverified(
                    "dependencies.installation",
                    "dependencies.layout_unverified",
                    reason,
                    file,
                ));
                return findings;
            }
            Ok(false) => (),
        }
    }
    for (name, dev) in &plan.packages {
        let id = format!("dependencies.package.{name}");
        if *dev && dev_omitted {
            findings.push(CheckResult::new(&id, Status::Skipped, "dependencies.dev_omitted", format!("{name} is a development dependency; pnpm records a production-only installation."), METADATA)
                .next("Install development dependencies outside Repo Doctor when preparing a local development environment."));
            continue;
        }
        let path = format!("node_modules/{name}/package.json");
        if let Err(reason) = reader.boundary(&path).await {
            findings.push(unverified(
                &id,
                "dependencies.package_unverified",
                reason,
                "package.json",
            ));
            continue;
        }
        findings.push(match reader.read(&path).await {
            Ok(None) => CheckResult::new(&id, Status::Fail, "dependencies.package_missing", format!("{name} has no installed package manifest in this directory's node_modules."), "package.json")
                .next("Install the declared dependencies with the project's package manager outside Repo Doctor, then check again. Review script approvals separately."),
            Ok(Some(input)) => match serde_json::from_str::<Value>(&input) {
                Ok(value) if value.is_object() => CheckResult::new(&id, Status::Pass, "dependencies.package_present", format!("{name} has an installed package manifest; package code was not loaded."), &path),
                _ => unverified(&id, "dependencies.package_unverified", "The installed package manifest is not a valid JSON object.", &path),
            },
            Err(reason) => unverified(&id, "dependencies.package_unverified", reason, &path),
        });
    }
    findings
}
