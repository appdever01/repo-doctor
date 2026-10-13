use serde::Serialize;

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pass,
    Warn,
    Fail,
    Skipped,
    Error,
}

#[derive(Serialize, Clone)]
pub struct Evidence {
    pub source: String,
    pub detail: String,
}

#[derive(Serialize, Clone)]
pub struct CheckResult {
    pub id: String,
    pub status: Status,
    pub finding_code: String,
    pub message: String,
    pub evidence: Vec<Evidence>,
    pub next_step: Option<String>,
    #[serde(skip)]
    pub evaluated: bool,
}

impl CheckResult {
    pub fn new(
        id: &str,
        status: Status,
        code: &str,
        message: impl Into<String>,
        source: &str,
    ) -> Self {
        Self {
            id: id.into(),
            status,
            finding_code: code.into(),
            message: message.into(),
            evidence: vec![Evidence {
                source: source.into(),
                detail: "Requirement or observation source".into(),
            }],
            next_step: None,
            evaluated: !matches!(status, Status::Skipped | Status::Error),
        }
    }

    pub fn incomplete(mut self) -> Self {
        self.evaluated = false;
        self
    }

    pub fn next(mut self, step: impl Into<String>) -> Self {
        self.next_step = Some(step.into());
        self
    }
}

#[derive(Default, Serialize)]
pub struct Summary {
    pub pass: usize,
    pub warn: usize,
    pub fail: usize,
    pub skipped: usize,
    pub error: usize,
}

#[derive(Serialize)]
pub struct Project {
    pub name: String,
    pub path: String,
    pub runtimes: Vec<String>,
}

#[derive(Serialize)]
pub struct Report {
    pub schema_version: u8,
    pub tool_version: String,
    pub project: Project,
    pub outcome: String,
    pub complete: bool,
    pub exit_code: u8,
    pub summary: Summary,
    pub checks: Vec<CheckResult>,
}

impl Report {
    pub fn build(
        mut checks: Vec<CheckResult>,
        runtimes: Vec<String>,
        strict: bool,
        input_error: bool,
    ) -> Self {
        checks.sort_by(|a, b| a.id.cmp(&b.id));
        for check in &mut checks {
            check.message = crate::safety::safe_text(&check.message);
            check.next_step = check.next_step.as_deref().map(crate::safety::safe_text);
            for evidence in &mut check.evidence {
                evidence.source = crate::safety::safe_text(&evidence.source);
                evidence.detail = crate::safety::safe_text(&evidence.detail);
            }
        }
        let complete = !input_error && checks.iter().all(|c| c.evaluated);
        let mut summary = Summary::default();
        for check in &checks {
            match check.status {
                Status::Pass => summary.pass += 1,
                Status::Warn => summary.warn += 1,
                Status::Fail => summary.fail += 1,
                Status::Skipped => summary.skipped += 1,
                Status::Error => summary.error += 1,
            }
        }
        let issues = summary.fail + summary.error > 0;
        let outcome = if input_error {
            "invalid_input"
        } else if issues {
            "issues_found"
        } else if !complete {
            "incomplete"
        } else {
            "passed"
        };
        let exit_code = if input_error {
            2
        } else if issues || (strict && (summary.warn > 0 || !complete)) {
            1
        } else {
            0
        };
        Self {
            schema_version: 1,
            tool_version: env!("CARGO_PKG_VERSION").into(),
            project: Project {
                name: "selected project".into(),
                path: ".".into(),
                runtimes,
            },
            outcome: outcome.into(),
            complete,
            exit_code,
            summary,
            checks,
        }
    }

    pub fn internal() -> Self {
        let mut report = Self::build(
            vec![CheckResult::new(
                "internal",
                Status::Error,
                "internal.failure",
                "An internal failure prevented a reliable report.",
                "repo-doctor",
            )],
            vec![],
            false,
            false,
        );
        report.outcome = "internal_error".into();
        report.exit_code = 3;
        report
    }
}
