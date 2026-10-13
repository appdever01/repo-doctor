use crate::model::{CheckResult, Status};
use crate::runner::{Host, Observation};
use node_semver::{Range, Version as NodeVersion};
use pep440_rs::{Version as PythonVersion, VersionSpecifiers};

pub enum Constraint {
    Node(Range),
    Python(VersionSpecifiers),
}

pub struct Requirement {
    pub source: String,
    pub display: String,
    pub constraint: Constraint,
}

impl Requirement {
    pub fn parse(
        kind: &str,
        input: &str,
        source: &str,
        version_file: bool,
    ) -> Result<Self, &'static str> {
        let value = input.trim();
        if value.is_empty()
            || value.len() > 256
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || b"vVabcdefrcpostdev+!.*~^<>=|,- \t".contains(&b))
        {
            return Err("Unsupported runtime requirement.");
        }
        let normalized = if version_file {
            let numeric = if kind == "node" {
                value.strip_prefix('v').unwrap_or(value)
            } else {
                value
            };
            let parts: Vec<_> = numeric.split('.').collect();
            if parts.is_empty()
                || parts.len() > 3
                || parts
                    .iter()
                    .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
            {
                return Err("Use a single numeric version in the version file.");
            }
            if kind == "python" {
                if parts.len() < 3 {
                    format!("=={numeric}.*")
                } else {
                    format!("=={numeric}")
                }
            } else {
                numeric.to_owned()
            }
        } else {
            value.to_owned()
        };
        let constraint = if kind == "node" {
            Constraint::Node(
                normalized
                    .parse()
                    .map_err(|_| "Unsupported npm version range.")?,
            )
        } else {
            Constraint::Python(
                normalized
                    .parse()
                    .map_err(|_| "Unsupported Python version specifiers.")?,
            )
        };
        Ok(Self {
            source: source.into(),
            display: normalized,
            constraint,
        })
    }

    pub fn matches(&self, version: &str) -> bool {
        match &self.constraint {
            Constraint::Node(range) => version
                .parse::<NodeVersion>()
                .is_ok_and(|v| v.satisfies(range)),
            Constraint::Python(spec) => version
                .parse::<PythonVersion>()
                .is_ok_and(|v| spec.contains(&v)),
        }
    }
}

pub struct Runtime {
    pub kind: &'static str,
    pub requirements: Vec<Requirement>,
    pub unsupported: Vec<String>,
}

impl Runtime {
    pub fn new(kind: &'static str) -> Self {
        Self {
            kind,
            requirements: vec![],
            unsupported: vec![],
        }
    }
    pub fn add(&mut self, value: &str, source: &str, file: bool) {
        match Requirement::parse(self.kind, value, source, file) {
            Ok(r) => self.requirements.push(r),
            Err(_) => self.unsupported.push(source.into()),
        }
    }
}

pub async fn check(runtime: &Runtime, host: &mut Host, static_mode: bool) -> Vec<CheckResult> {
    let kind = runtime.kind;
    let label = if kind == "node" { "Node.js" } else { "Python" };
    let id = format!("runtime.{kind}");
    let observation = if static_mode {
        Observation::Unverified("Static mode does not execute runtimes.")
    } else {
        host.runtime(kind).await
    };
    let presence = match &observation {
        Observation::Version(v) => CheckResult::new(&id, Status::Pass, "runtime.present", format!("{label} {v} is present."), "host PATH"),
        Observation::Missing => CheckResult::new(&id, Status::Fail, "runtime.missing", format!("{label} is absent from PATH."), "host PATH").next(format!("Install or select a host {label} runtime, then check again.")),
        Observation::Unverified(reason) => CheckResult::new(&id, Status::Skipped, "runtime.unverified", format!("{label} could not be verified. {reason}"), "host PATH").next("Select an absolute host runtime outside the project; use numeric version requirements."),
        Observation::Error(reason) => CheckResult::new(&id, Status::Error, "runtime.probe_error", format!("{label} version could not be inspected. {reason}"), "host PATH").next("Check the selected host runtime, then run Repo Doctor again."),
    };
    let mut checks = vec![presence];
    for (i, requirement) in runtime.requirements.iter().enumerate() {
        let req_id = format!("{id}.requirement.{i}");
        checks.push(match &observation {
            Observation::Version(v) if requirement.matches(v) => CheckResult::new(
                &req_id,
                Status::Pass,
                "runtime.version_matches",
                format!("{label} {v} satisfies {}.", requirement.display),
                &requirement.source,
            ),
            Observation::Version(v) => CheckResult::new(
                &req_id,
                Status::Fail,
                "runtime.version_mismatch",
                format!("{label} {v} does not satisfy {}.", requirement.display),
                &requirement.source,
            )
            .next("Select a host runtime that satisfies every declared requirement."),
            _ => CheckResult::new(
                &req_id,
                Status::Skipped,
                "runtime.prerequisite_unverified",
                format!(
                    "{label} compatibility is unverified because no usable version was observed."
                ),
                &requirement.source,
            ),
        });
    }
    for (i, source) in runtime.unsupported.iter().enumerate() {
        checks.push(
            CheckResult::new(
                &format!("{id}.unsupported.{i}"),
                Status::Warn,
                "runtime.unsupported_requirement",
                format!("{label} has an unsupported version declaration."),
                source,
            )
            .incomplete()
            .next("Replace aliases or unsupported syntax with a numeric requirement."),
        );
    }
    if runtime.requirements.is_empty() && runtime.unsupported.is_empty() {
        checks.push(
            CheckResult::new(
                &format!("{id}.compatibility"),
                Status::Warn,
                "runtime.requirement_missing",
                format!(
                    "No {label} version requirement was declared; compatibility is unverified."
                ),
                "project declarations",
            )
            .incomplete()
            .next("Declare a supported runtime version requirement."),
        );
    }
    checks
}
