use crate::checks::{dependencies, environment, runtime, sockets};
use crate::cli::Options;
use crate::config::{Config, Endpoint};
use crate::discovery::discover;
use crate::model::{CheckResult, Report, Status};
use crate::runner::Host;
use crate::safety::{Cancellation, CancellationListener, FileReader, RUN_LIMIT, within_deadline};
use std::sync::Arc;
use std::time::Instant;

pub async fn check(options: &Options) -> Report {
    let deadline = Instant::now() + RUN_LIMIT;
    let cancelled = Arc::new(Cancellation::default());
    let observed = cancelled.clone();
    let _listener = CancellationListener(tokio::spawn(async move {
        crate::runner::cancellation().await;
        observed.cancel();
    }));
    tokio::task::yield_now().await;
    let target = options.path.clone();
    let root = match within_deadline(deadline, &cancelled, move || {
        target.canonicalize().ok().filter(|root| root.is_dir())
    })
    .await
    {
        Ok(Some(root)) => root,
        Err(reason) => return execution_error(reason, options.strict),
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
    reader.deadline = deadline;
    reader.cancelled = cancelled.clone();
    let (config, configured) = match reader.read(".repo-doctor.toml").await {
        Ok(Some(text)) => match Config::parse(&text) {
            Ok(config) => (config, true),
            Err(reason) => return invalid_config(reason, options.strict),
        },
        Ok(None) => (Config::default(), false),
        Err(reason) if cancelled.is_cancelled() || Instant::now() >= deadline => {
            return execution_error(reason, options.strict);
        }
        Err(reason) => return invalid_config(reason, options.strict),
    };
    let mut host = Host {
        root,
        path: std::env::var_os("PATH"),
        deadline: reader.deadline,
        cancelled: false,
        filesystem_cancelled: cancelled.clone(),
    };
    for path in config.env.template.iter().chain(&config.env.files) {
        if let Err(reason) = reader.boundary(path).await {
            if cancelled.is_cancelled() || Instant::now() >= deadline {
                return execution_error(reason, options.strict);
            }
            return invalid_config(reason, options.strict);
        }
    }
    let mut project = discover(&mut reader, &config, configured, &host, options.static_mode).await;
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
        .extend(environment::check(&mut reader, &config.env).await);
    if let Some(plan) = &project.dependencies {
        project
            .findings
            .extend(dependencies::check(&mut reader, plan).await);
    }
    host.cancelled |= cancelled.is_cancelled();
    let kinds = project.runtimes.iter().map(|r| r.kind.to_owned()).collect();
    for runtime in &project.runtimes {
        project
            .findings
            .extend(runtime::check(runtime, &mut host, options.static_mode).await);
    }
    host.cancelled |= cancelled.is_cancelled();
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
    if host.cancelled || cancelled.is_cancelled() || Instant::now() >= host.deadline {
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

fn execution_error(reason: &str, strict: bool) -> Report {
    Report::build(
        vec![CheckResult::new(
            "execution.incomplete",
            Status::Error,
            "execution.incomplete",
            reason,
            "execution policy",
        )],
        vec![],
        strict,
        false,
    )
}

async fn endpoint_check(
    endpoint: &Endpoint,
    service: bool,
    enabled: bool,
    host: &mut Host,
) -> CheckResult {
    host.cancelled |= host.filesystem_cancelled.is_cancelled();
    if !enabled || host.cancelled {
        return sockets::check(endpoint, service, false, host.deadline).await;
    }
    tokio::select! {
        result = sockets::check(endpoint, service, true, host.deadline) => result,
        _ = host.filesystem_cancelled.wait() => {
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
