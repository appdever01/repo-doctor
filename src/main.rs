use clap::Parser;
use repo_doctor::{
    cli::{Cli, Commands, Format},
    model::Report,
    output,
};
use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let Cli {
        command: Commands::Check(options),
    } = Cli::parse();
    std::panic::set_hook(Box::new(|_| {}));
    let report = std::panic::catch_unwind(|| {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime.block_on(repo_doctor::app::check(&options)),
            Err(_) => Report::internal(),
        }
    })
    .unwrap_or_else(|_| Report::internal());
    let rendered = match options.format {
        Format::Human => output::human(&report),
        Format::Json => match output::json(&report) {
            Ok(value) => value + "\n",
            Err(_) => return ExitCode::from(3),
        },
    };
    if std::io::stdout()
        .lock()
        .write_all(rendered.as_bytes())
        .is_err()
    {
        return ExitCode::from(3);
    }
    ExitCode::from(report.exit_code)
}
