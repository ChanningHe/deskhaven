//! deskhaven server binary.

use std::io::Write;
use std::process::ExitCode;

use clap::Parser;
use deskhaven::cli::{Cli, Command, ConfigCommand, KeyCommand};
use deskhaven::{config_cmd, key_cmd, log_safe, serve};

fn main() -> ExitCode {
    let result = deskhaven::install_crypto_provider().and_then(|()| run(&Cli::parse()));
    match result {
        Ok(code) => code,
        Err(err) => {
            // Nothing sensible remains if stderr itself is gone.
            let _ = writeln!(
                std::io::stderr().lock(),
                "error: {}",
                log_safe::render(&err)
            );
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> anyhow::Result<ExitCode> {
    let options = cli.load_options()?;
    let done = |result: anyhow::Result<()>| result.map(|()| ExitCode::SUCCESS);
    match cli.command.as_ref().unwrap_or(&Command::Serve) {
        // Logs its own failures once logging is up.
        Command::Serve => Ok(serve::run(&options)),
        Command::Config(ConfigCommand::Check) => done(config_cmd::check(&options)),
        Command::Config(ConfigCommand::Print { effective }) => {
            done(config_cmd::print(&options, *effective))
        }
        Command::Config(ConfigCommand::Schema) => done(config_cmd::schema()),
        Command::Key(KeyCommand::Show) => done(key_cmd::show(&options)),
    }
}
