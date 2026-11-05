//! `docs/reference/cli.md` is written by hand; this keeps it and the clap definition in
//! step, in both directions:
//!
//! - every visible leaf command has a `### `deskhaven …`` section and every section names a
//!   real command;
//! - each section documents exactly the command's own options and positionals and those of
//!   its parent commands (global ones excluded);
//! - the Global options table documents exactly the root's options.
//!
//! An option is its `--long` name, else its `-s` short name; a positional is `<NAME>` (its
//! value name, else its id in capitals). Tokens are matched whole; fenced code blocks
//! (example shell commands) are not read.

use std::collections::BTreeSet;

use clap::{Arg, Command, CommandFactory};
use deskhaven::cli::Cli;

const REFERENCE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/reference/cli.md");
const GLOBAL_HEADING: &str = "## Global options";
const COMMAND_PREFIX: &str = "### `";
const FENCE: &str = "```";
/// clap's own arguments on every subcommand.
const AUTO_ARGS: [&str; 2] = ["help", "version"];

type Items = BTreeSet<String>;

// --- clap side ---

fn item(arg: &Arg) -> Option<String> {
    if arg.is_positional() {
        let name = arg
            .get_value_names()
            .and_then(|names| names.first())
            .map_or_else(|| arg.get_id().as_str().to_uppercase(), |n| n.to_string());
        return Some(format!("<{name}>"));
    }
    match (arg.get_long(), arg.get_short()) {
        (Some(long), _) => Some(format!("--{long}")),
        (None, Some(short)) => Some(format!("-{short}")),
        (None, None) => None,
    }
}

/// The visible, non-global arguments of `command` itself.
fn own_items(command: &Command) -> Items {
    command
        .get_arguments()
        .filter(|arg| !arg.is_hide_set() && !arg.is_global_set())
        .filter(|arg| !AUTO_ARGS.contains(&arg.get_id().as_str()))
        .filter_map(item)
        .collect()
}

/// `command path -> items` for every visible leaf, parents' items included.
fn clap_commands(command: &Command, path: &str, inherited: &Items, out: &mut Vec<(String, Items)>) {
    let items: Items = inherited.union(&own_items(command)).cloned().collect();
    let subcommands: Vec<&Command> = command
        .get_subcommands()
        .filter(|sub| !sub.is_hide_set() && sub.get_name() != "help")
        .collect();
    if subcommands.is_empty() {
        out.push((path.to_owned(), items));
        return;
    }
    for sub in subcommands {
        clap_commands(sub, &format!("{path} {}", sub.get_name()), &items, out);
    }
}

fn clap_globals(root: &Command) -> Items {
    root.get_arguments()
        .filter(|arg| !arg.is_hide_set())
        .filter_map(item)
        .collect()
}

// --- reference side ---

/// `(heading, body lines)` per `##` / `###` section, fenced code blocks dropped.
fn sections(text: &str) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    let mut in_fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with(FENCE) {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if line.starts_with("## ") || line.starts_with("### ") {
            out.push((line.to_owned(), Vec::new()));
        } else if let Some((_, body)) = out.last_mut() {
            body.push(line.to_owned());
        }
    }
    out
}

fn is_option(token: &str) -> bool {
    let long = token.strip_prefix("--").is_some_and(|rest| {
        rest.starts_with(|c: char| c.is_ascii_alphanumeric())
            && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    });
    let short = token.len() == 2
        && token.starts_with('-')
        && token[1..].starts_with(|c: char| c.is_ascii_alphanumeric());
    long || short
}

fn is_placeholder(token: &str) -> bool {
    token.len() > 2
        && token.starts_with('<')
        && token.ends_with('>')
        && token[1..token.len() - 1]
            .chars()
            .all(|c| c.is_ascii_uppercase() || c == '_')
}

/// Whole option and positional tokens: `--long`, `-s`, `<NAME>`. A `<NAME>` right after an
/// option is that option's value, not a positional.
fn tokens(text: &str) -> Items {
    let words = text
        .split(|c: char| !(c.is_ascii_alphanumeric() || "-_<>".contains(c)))
        .filter(|word| !word.is_empty());
    let mut previous = "";
    let mut out = Items::new();
    for word in words {
        if is_option(word) || (is_placeholder(word) && !is_option(previous)) {
            out.insert(word.to_owned());
        }
        previous = word;
    }
    out
}

/// `command path -> documented items` per command section. The heading is exactly one code
/// span: the path, then optionally positionals and options.
fn documented_commands(text: &str) -> Result<Vec<(String, Items)>, String> {
    sections(text)
        .into_iter()
        .filter_map(|(heading, body)| {
            let span = heading.strip_prefix(COMMAND_PREFIX)?;
            let Some(span) = span.strip_suffix('`') else {
                return Some(Err(format!("heading is not one code span: {heading}")));
            };
            let path: Vec<&str> = span
                .split(' ')
                .take_while(|word| !word.starts_with(['<', '-', '[']))
                .collect();
            let items: Items = tokens(span)
                .union(&tokens(&body.join("\n")))
                .cloned()
                .collect();
            Some(Ok((path.join(" "), items)))
        })
        .collect()
}

/// Options in the first column of the Global options table.
fn documented_globals(text: &str) -> Result<Items, String> {
    let (_, body) = sections(text)
        .into_iter()
        .find(|(heading, _)| heading == GLOBAL_HEADING)
        .ok_or_else(|| format!("no `{GLOBAL_HEADING}` section"))?;
    Ok(body
        .iter()
        .filter_map(|line| line.strip_prefix('|')?.split('|').next())
        .flat_map(tokens)
        .collect())
}

fn check(reference: &str) -> Result<(), String> {
    let mut root = Cli::command();
    root.build();
    let mut expected = Vec::new();
    clap_commands(&root, root.get_name(), &Items::new(), &mut expected);
    let documented = documented_commands(reference)?;

    let mut problems = Vec::new();
    for (path, items) in &expected {
        match documented.iter().find(|(doc_path, _)| doc_path == path) {
            None => problems.push(format!("`{path}` has no section")),
            Some((_, doc)) if doc != items => problems.push(format!(
                "`{path}`: clap has {items:?}, the reference documents {doc:?}"
            )),
            Some(_) => {}
        }
    }
    for (path, _) in &documented {
        if !expected.iter().any(|(clap_path, _)| clap_path == path) {
            problems.push(format!("`{path}` is documented but is no command"));
        }
    }
    let (globals, doc_globals) = (clap_globals(&root), documented_globals(reference)?);
    if globals != doc_globals {
        problems.push(format!(
            "global options: clap has {globals:?}, the reference documents {doc_globals:?}"
        ));
    }
    match problems.is_empty() {
        true => Ok(()),
        false => Err(problems.join("\n")),
    }
}

#[test]
fn the_cli_reference_matches_the_clap_definition() {
    let reference = std::fs::read_to_string(REFERENCE).unwrap();

    check(&reference).unwrap_or_else(|problems| panic!("docs/reference/cli.md:\n{problems}"));
}

#[test]
fn the_check_catches_missing_extra_and_misnamed_items() {
    let reference = std::fs::read_to_string(REFERENCE).unwrap();
    let mutations = [
        ("- `--effective`:", "- `--effectivex`:"),
        ("- `--effective`:", "- `--effective` `--all`:"),
        (
            "### `deskhaven config check`",
            "### `deskhaven config check-all`",
        ),
        (
            "### `deskhaven healthcheck`",
            "### `deskhaven healthcheck` `--port`",
        ),
        ("<USERNAME>", "<NAME>"),
        ("| `--data-dir <PATH>` |", "| `--data <PATH>` |"),
    ];
    for (from, to) in mutations {
        assert!(reference.contains(from), "mutation target `{from}` is gone");
        let mutated = reference.replace(from, to);
        assert!(check(&mutated).is_err(), "not caught: `{from}` -> `{to}`");
    }
}
