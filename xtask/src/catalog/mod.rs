//! `xtask gen-strategy-catalog [--from <dir>] [--check]`: regenerate the client option
//! catalog of the strategy editor (`deskhaven_core::strategies::catalog`) from the client
//! source, merged with the human-written [`OVERLAY`]; `--check` fails when the committed
//! [`GENERATED`] differs.
//!
//! The source is [`VENDOR`] unless `--from` names another directory. It holds the two files
//! read (as text, never built), at their repository paths, each with a
//! `// vendored-revision: <commit>` line that must match the overlay's `[source]`:
//!
//! ```text
//! <dir>/hbb_common/src/config.rs                  rustdesk/hbb_common at the client's pin
//! <dir>/rustdesk/libs/base/src/config/keys.rs     rustdesk/rustdesk
//! ```
//!
//! To move to a newer client: fetch the two files (e.g. `curl -sfL
//! https://raw.githubusercontent.com/rustdesk/hbb_common/<rev>/src/config.rs`), trim them
//! like the vendored copies (keep `option2bool`, the `keys` module and the key lists), update
//! their header and revision line and the overlay's `[source]`, then regenerate.

mod overlay;
mod render;
mod source;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::error::XtaskError;

/// The human-written half, relative to the repository root.
pub const OVERLAY: &str = "crates/deskhaven-core/src/strategies/catalog/overlay.toml";
/// The generated module, relative to the repository root.
pub const GENERATED: &str = "crates/deskhaven-core/src/strategies/catalog/generated.rs";
/// The vendored client source, relative to the repository root.
pub const VENDOR: &str = "xtask/vendor/strategy-catalog";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// `None`: [`VENDOR`].
    pub from: Option<PathBuf>,
    pub check: bool,
}

impl Args {
    pub fn parse(args: &[String]) -> Result<Self, XtaskError> {
        let mut from = None;
        let mut check = false;
        let mut rest = args.iter();
        while let Some(arg) = rest.next() {
            match arg.as_str() {
                "--check" => check = true,
                "--from" => {
                    let dir = rest
                        .next()
                        .ok_or(XtaskError::MissingArgument("--from <dir>"))?;
                    from = Some(PathBuf::from(dir));
                }
                other => return Err(XtaskError::UnknownArgument(other.to_owned())),
            }
        }
        Ok(Self { from, check })
    }
}

pub fn run(root: &Path, args: &Args) -> Result<(), XtaskError> {
    let from = args.from.clone().unwrap_or_else(|| root.join(VENDOR));
    let generated = generate(&from, &read(&root.join(OVERLAY))?)?;
    let path = root.join(GENERATED);
    let current = match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(source) => return Err(io_error(&path, source)),
    };
    if current.as_deref() == Some(generated.as_str()) {
        eprintln!("xtask: {GENERATED} is up to date");
        return Ok(());
    }
    if args.check {
        return Err(XtaskError::CatalogDrift(GENERATED.to_owned()));
    }
    fs::write(&path, &generated).map_err(|source| io_error(&path, source))?;
    eprintln!("xtask: {GENERATED} updated");
    Ok(())
}

/// The `generated.rs` text for the sources in `from` and the overlay text.
fn generate(from: &Path, overlay: &str) -> Result<String, XtaskError> {
    let config_rs = read(&from.join(source::CONFIG_RS))?;
    let keys_rs = read(&from.join(source::KEYS_RS))?;
    let keys = source::settings_keys(&config_rs, &keys_rs).map_err(catalog_error)?;
    let catalog = overlay::merge(&keys, overlay).map_err(catalog_error)?;
    check_revision(source::CONFIG_RS, &config_rs, &catalog.source.hbb_common)?;
    check_revision(source::KEYS_RS, &keys_rs, &catalog.source.rustdesk)?;
    Ok(render::render(&catalog))
}

/// The file's `// vendored-revision:` line names the commit the overlay records.
fn check_revision(file: &str, src: &str, recorded: &str) -> Result<(), XtaskError> {
    match source::revision(src) {
        Some(rev) if rev == recorded => Ok(()),
        Some(rev) => Err(XtaskError::Catalog(format!(
            "{file} is from {rev} but overlay.toml [source] records {recorded}"
        ))),
        None => Err(XtaskError::Catalog(format!(
            "{file} has no `// vendored-revision:` line"
        ))),
    }
}

fn catalog_error(err: impl ToString) -> XtaskError {
    XtaskError::Catalog(err.to_string())
}

fn read(path: &Path) -> Result<String, XtaskError> {
    fs::read_to_string(path).map_err(|source| io_error(path, source))
}

fn io_error(path: &Path, source: io::Error) -> XtaskError {
    XtaskError::Io {
        path: path.display().to_string(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixture's credential-like key must be described.
    const OVERLAY_TEXT: &str = "[source]\nrustdesk = \"fixture-rustdesk\"\nhbb_common = \"fixture-hbb\"\n\
        [options.proxy-password]\ngroup = \"network\"\nkind = \"text\"\nsecret = true\n\
        label = { en = \"P\", zh = \"P\" }\ndescription = { en = \"\", zh = \"\" }\n";

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/strategy-catalog")
    }

    #[test]
    fn parses_from_and_check() {
        let parsed = Args::parse(&args(&["--check", "--from", "src"])).unwrap();
        assert_eq!(
            parsed,
            Args {
                from: Some(PathBuf::from("src")),
                check: true
            }
        );
        let vendored = Args::parse(&args(&["--check"])).unwrap();
        assert_eq!(vendored.from, None);
        assert!(matches!(
            Args::parse(&args(&["--from"])),
            Err(XtaskError::MissingArgument(_))
        ));
        assert!(matches!(
            Args::parse(&args(&["--from", "x", "--fix"])),
            Err(XtaskError::UnknownArgument(_))
        ));
    }

    #[test]
    fn generates_from_the_fixture_sources_and_reports_bad_input() {
        let out = generate(&fixture(), OVERLAY_TEXT).unwrap();

        assert!(out.contains("key: \"enable-keyboard\""), "{out}");
        assert!(out.contains("key: \"disable-udp\""));
        assert!(
            !out.contains("\"theme\""),
            "local settings are not strategy options"
        );
        assert!(matches!(
            generate(&fixture().join("nope"), OVERLAY_TEXT),
            Err(XtaskError::Io { .. })
        ));
        assert!(matches!(
            generate(&fixture(), "[source]"),
            Err(XtaskError::Catalog(_))
        ));
    }

    #[test]
    fn the_vendored_revisions_must_match_the_overlay() {
        let stale = OVERLAY_TEXT.replace("fixture-hbb", "abc1234");

        let err = generate(&fixture(), &stale).unwrap_err().to_string();

        assert!(
            err.contains("fixture-hbb") && err.contains("abc1234"),
            "{err}"
        );
        assert!(check_revision("f", "no marker", "r").is_err());
        assert!(check_revision("f", "// vendored-revision: r\n", "r").is_ok());
    }

    #[test]
    fn check_detects_drift_and_write_fixes_it() {
        let root = tempfile::tempdir().unwrap();
        let overlay = root.path().join(OVERLAY);
        fs::create_dir_all(overlay.parent().unwrap()).unwrap();
        fs::write(&overlay, OVERLAY_TEXT).unwrap();
        let check = Args {
            from: Some(fixture()),
            check: true,
        };
        let write = Args {
            check: false,
            ..check.clone()
        };

        assert!(matches!(
            run(root.path(), &check),
            Err(XtaskError::CatalogDrift(_))
        ));
        run(root.path(), &write).unwrap();
        run(root.path(), &check).unwrap();
        fs::write(root.path().join(GENERATED), "edited by hand").unwrap();
        assert!(matches!(
            run(root.path(), &check),
            Err(XtaskError::CatalogDrift(_))
        ));
    }

    #[test]
    fn the_committed_catalog_is_what_the_vendored_source_generates() {
        let root = crate::repo_root();
        let overlay = read(&root.join(OVERLAY)).unwrap();

        let generated = generate(&root.join(VENDOR), &overlay).unwrap();

        assert_eq!(read(&root.join(GENERATED)).unwrap(), generated);
    }
}
