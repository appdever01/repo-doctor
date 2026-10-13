use crate::checks::{environment, runtime, sockets};
use crate::cli::Options;
use crate::config::{Config, Endpoint};
use crate::discovery::discover;
use crate::model::{CheckResult, Report, Status};
use crate::runner::Host;
use crate::safety::FileReader;

pub async fn check(options: &Options) -> Report {
    let root = match options.path.canonicalize() {
        Ok(root) if root.is_dir() => root,
        _ => {
            return Report::build(
                vec![CheckResult::new(
                    "input.target",
                    Status::Error,
                    "input.target_unusable",
                    "The selected target must be an accessible directory.",
                    "selected target",
                )],
                vec![],
                options.strict,
                true,
            );
        }
    };
    let mut reader = FileReader::new(root.clone());
    let (config, configured) = match reader.read(".repo-doctor.toml") {
        Ok(Some(text)) => match Config::parse(&text) {
            Ok(config) => (config, true),
            Err(reason) => return invalid_config(reason, options.strict),
        },
        Ok(None) => (Config::default(), false),
        Err(reason) => return invalid_config(reason, options.strict),
    };
    let mut host = Host {
        root,
        path: std::env::var_os("PATH"),
        deadline: reader.deadline,
        cancelled: false,
    };
    for path in config.env.template.iter().chain(&config.env.files) {
        if let Err(reason) = reader.boundary(path) {
            return invalid_config(reason, options.strict);
        }
    }
    let mut project = discover(&mut reader, &config, configured, &host, options.static_mode);
    if !project.supported {
        project.findings.push(
            CheckResult::new(
                "project.unsupported",
                Status::Warn,
                "project.unsupported",
                "No supported project declaration or configuration was found.",
                "selected directory",
            )
            .incomplete()
            .next("Select the directory containing a supported manifest or add .repo-doctor.toml."),
        );
    }
    project
        .findings
        .extend(environment::check(&mut reader, &config.env));
    let kinds = project.runtimes.iter().map(|r| r.kind.to_owned()).collect();
    for runtime in &project.runtimes {
        project
            .findings
            .extend(runtime::check(runtime, &mut host, options.static_mode).await);
    }
    for endpoint in &config.ports {
        project.findings.push(
            endpoint_check(
                endpoint,
                false,
                !options.static_mode && !host.cancelled,
                &mut host,
            )
            .await,
        );
    }
    for endpoint in &config.services {
        project.findings.push(
            endpoint_check(
                endpoint,
                true,
                options.services && !options.static_mode && !host.cancelled,
                &mut host,
            )
            .await,
        );
    }
    if host.cancelled || std::time::Instant::now() >= host.deadline {
        project.findings.push(CheckResult::new(
            "execution.incomplete",
            Status::Error,
            "execution.incomplete",
            "The run was cancelled or reached its 15-second deadline; coverage is incomplete.",
            "execution policy",
        ));
    }
    Report::build(project.findings, kinds, options.strict, false)
}

async fn endpoint_check(
    endpoint: &Endpoint,
    service: bool,
    enabled: bool,
    host: &mut Host,
) -> CheckResult {
    if !enabled {
        return sockets::check(endpoint, service, false, host.deadline).await;
    }
    tokio::select! {
        result = sockets::check(endpoint, service, true, host.deadline) => result,
        _ = crate::runner::cancellation() => {
            host.cancelled = true;
            CheckResult::new(&format!("{}.{}", if service { "service" } else { "port" }, endpoint.name), Status::Skipped, "execution.cancelled", "The configured socket check was cancelled.", ".repo-doctor.toml")
        }
    }
}

fn invalid_config(reason: &str, strict: bool) -> Report {
    Report::build(
        vec![
            CheckResult::new(
                "input.config",
                Status::Error,
                "input.config_invalid",
                reason,
                ".repo-doctor.toml",
            )
            .next("Correct the configuration using schema 1 and the documented fields."),
        ],
        vec![],
        strict,
        true,
    )
}
