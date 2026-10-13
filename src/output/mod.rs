use crate::model::Report;

pub fn human(report: &Report) -> String {
    let mut text = format!("Repo Doctor: {}\n\n", report.project.name);
    for check in &report.checks {
        let status = match check.status {
            crate::model::Status::Pass => "PASS",
            crate::model::Status::Warn => "WARN",
            crate::model::Status::Fail => "FAIL",
            crate::model::Status::Skipped => "SKIP",
            crate::model::Status::Error => "ERROR",
        };
        text.push_str(&format!("{status:<5} {}\n", check.message));
        for evidence in &check.evidence {
            text.push_str(&format!("      Source: {}\n", evidence.source));
        }
        if let Some(step) = &check.next_step {
            text.push_str(&format!("      Next: {step}\n"));
        }
        text.push('\n');
    }
    let summary = &report.summary;
    text.push_str(&format!(
        "{} passed, {} warnings, {} failed, {} skipped, {} errors\n",
        summary.pass, summary.warn, summary.fail, summary.skipped, summary.error
    ));
    let outcome = match report.outcome.as_str() {
        "passed" => "declared setup checks passed",
        "issues_found" => "declared setup issues found",
        "invalid_input" => "invalid input",
        "internal_error" => "internal failure",
        _ => "declared setup remains unverified",
    };
    text.push_str(&format!(
        "Result: {outcome}; coverage {}\nExit code: {}\n",
        if report.complete {
            "complete"
        } else {
            "incomplete"
        },
        report.exit_code
    ));
    text
}

pub fn json(report: &Report) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(report)
}
