//! deskhaven server binary.

mod cli;
mod config_cmd;

use std::io::Write;
use std::process::ExitCode;

use clap::Parser;

use cli::{Cli, Command, ConfigCommand};

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // Nothing sensible remains if stderr itself is gone.
            let _ = writeln!(std::io::stderr().lock(), "error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> anyhow::Result<()> {
    let options = cli.load_options()?;
    match cli.command.as_ref().unwrap_or(&Command::Serve) {
        Command::Serve => serve(&options),
        Command::Config(ConfigCommand::Check) => config_cmd::check(&options),
        Command::Config(ConfigCommand::Print { effective }) => {
            config_cmd::print(&options, *effective)
        }
        Command::Config(ConfigCommand::Schema) => config_cmd::schema(),
    }
}

/// Startup currently stops after configuration; subsystems attach here.
fn serve(options: &deskhaven_config::bootstrap::LoadOptions) -> anyhow::Result<()> {
    config_cmd::load_reporting(options).map(drop)
}
