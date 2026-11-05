//! `docs/reference/cli.md` is written by hand; this keeps it from drifting from the clap
//! definition: every command has its section, and every option and argument is named.

use clap::CommandFactory;
use deskhaven::cli::Cli;

const REFERENCE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/reference/cli.md");

/// `(command path, its options and arguments as documented)` for every leaf command.
fn leaves(command: &clap::Command, path: &str, out: &mut Vec<(String, Vec<String>)>) {
    let subcommands: Vec<_> = command
        .get_subcommands()
        .filter(|sub| sub.get_name() != "help")
        .collect();
    if subcommands.is_empty() {
        out.push((path.to_owned(), names(command)));
    }
    for sub in subcommands {
        leaves(sub, &format!("{path} {}", sub.get_name()), out);
    }
}

/// `--flag` for options, `<NAME>` for positional arguments; global ones are left to the
/// root.
fn names(command: &clap::Command) -> Vec<String> {
    command
        .get_arguments()
        .filter(|arg| !arg.is_global_set())
        .filter_map(|arg| match arg.get_long() {
            Some(long) => Some(format!("--{long}")),
            None if arg.is_positional() => Some(format!(
                "<{}>",
                arg.get_value_names()?.first()?.as_str().to_owned()
            )),
            None => None,
        })
        .filter(|name| name != "--help" && name != "--version")
        .collect()
}

#[test]
fn every_command_option_and_argument_is_documented() {
    let reference = std::fs::read_to_string(REFERENCE).unwrap();
    let root = Cli::command();
    let mut commands = Vec::new();
    leaves(&root, root.get_name(), &mut commands);

    assert!(commands.len() > 5, "{commands:?}");
    for (path, names) in commands {
        let heading = format!("### `{path}");
        let section = reference
            .split("\n### ")
            .find(|s| format!("### {s}").starts_with(&heading))
            .unwrap_or_else(|| panic!("no section for `{path}` in docs/reference/cli.md"));
        for name in names {
            assert!(
                section.contains(&name),
                "`{path}`: {name} is not documented"
            );
        }
    }
    for global in root.get_arguments().filter_map(|arg| arg.get_long()) {
        assert!(
            reference.contains(&format!("`--{global}")),
            "global option --{global} is not documented"
        );
    }
}
