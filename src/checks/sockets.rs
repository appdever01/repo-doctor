use crate::config::Endpoint;
use crate::model::{CheckResult, Status};
use std::net::{SocketAddr, TcpListener};
use std::time::{Duration, Instant};

pub async fn check(
    endpoint: &Endpoint,
    service: bool,
    enabled: bool,
    deadline: Instant,
) -> CheckResult {
    let id = format!(
        "{}.{}",
        if service { "service" } else { "port" },
        endpoint.name
    );
    let source = ".repo-doctor.toml";
    if !enabled || Instant::now() >= deadline {
        return CheckResult::new(
            &id,
            Status::Skipped,
            "socket.skipped",
            "The configured socket check was skipped by execution policy or the run deadline.",
            source,
        )
        .next(if service {
            "Run with --services to attempt this TCP connection; omit --static."
        } else {
            "Run without --static to inspect this port."
        });
    }
    let address = SocketAddr::new(endpoint.address, endpoint.port);
    if service {
        let budget =
            Duration::from_millis(500).min(deadline.saturating_duration_since(Instant::now()));
        match tokio::time::timeout(budget, tokio::net::TcpStream::connect(address)).await {
            Ok(Ok(stream)) => {
                drop(stream);
                CheckResult::new(
                    &id,
                    Status::Pass,
                    "service.tcp_accepted",
                    format!("{}: TCP connection accepted at {address}.", endpoint.name),
                    source,
                )
            }
            Err(_) => failure(endpoint, true, None),
            Ok(Err(e)) => failure(endpoint, true, Some(e.kind())),
        }
    } else {
        match TcpListener::bind(address) {
            Ok(listener) => {
                drop(listener);
                CheckResult::new(
                    &id,
                    Status::Pass,
                    "port.available",
                    format!("{}: {address} was available when checked.", endpoint.name),
                    source,
                )
            }
            Err(e) => failure(endpoint, false, Some(e.kind())),
        }
    }
}

fn failure(endpoint: &Endpoint, service: bool, error: Option<std::io::ErrorKind>) -> CheckResult {
    use std::io::ErrorKind;
    let id = format!(
        "{}.{}",
        if service { "service" } else { "port" },
        endpoint.name
    );
    let address = SocketAddr::new(endpoint.address, endpoint.port);
    let (status, code, message, next) = match (service, error) {
        (true, None) | (true, Some(ErrorKind::TimedOut)) => (
            Status::Fail,
            "service.timeout",
            format!("{}: TCP connection timed out.", endpoint.name),
            Some("Start or inspect the intended local listener, then check again."),
        ),
        (true, Some(ErrorKind::ConnectionRefused)) => (
            Status::Fail,
            "service.refused",
            format!(
                "{}: TCP connection was refused at {address}.",
                endpoint.name
            ),
            Some("Start the intended local listener, then check again."),
        ),
        (false, Some(ErrorKind::AddrInUse)) => (
            Status::Fail,
            "port.occupied",
            format!("{}: {address} is already in use.", endpoint.name),
            Some("Review the process using this port or choose a different configured port."),
        ),
        (true, _) => (
            Status::Error,
            "service.inspection_error",
            "The TCP connection could not be inspected.".into(),
            Some("Review local socket access and address-family support."),
        ),
        (false, _) => (
            Status::Error,
            "port.inspection_error",
            "Port availability could not be inspected.".into(),
            Some("Review local socket access and address-family support."),
        ),
    };
    let mut result = CheckResult::new(&id, status, code, message, ".repo-doctor.toml");
    result.next_step = next.map(str::to_owned);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn timeout_and_permission_errors_cannot_pass() {
        let endpoint = Endpoint {
            name: "api".into(),
            address: "127.0.0.1".parse().unwrap(),
            port: 1234,
        };
        assert!(
            tokio::time::timeout(Duration::from_millis(1), std::future::pending::<()>())
                .await
                .is_err()
        );
        let result = failure(&endpoint, true, None);
        assert_eq!(result.status, Status::Fail);
        assert_eq!(result.finding_code, "service.timeout");
        for service in [true, false] {
            for kind in [
                std::io::ErrorKind::PermissionDenied,
                std::io::ErrorKind::AddrNotAvailable,
            ] {
                let result = failure(&endpoint, service, Some(kind));
                assert_eq!(result.status, Status::Error);
                assert!(!result.evaluated);
            }
        }
    }
}
