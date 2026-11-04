//! `xtask gen-strategy-catalog --from <dir> [--check]`: regenerate the client option catalog
//! of the strategy editor (`deskhaven_core::strategies::catalog`) from the client source in
//! `<dir>`, merged with the human-written [`OVERLAY`]; `--check` fails when the committed
//! [`GENERATED`] differs.
//!
//! `<dir>` holds the two files read (as text, never built), at their repository paths:
//!
//! ```text
//! <dir>/hbb_common/src/config.rs                  rustdesk/hbb_common at the client's pin
//! <dir>/rustdesk/libs/base/src/config/keys.rs     rustdesk/rustdesk
//! ```
//!
//! e.g. `curl -sfL -o <dir>/hbb_common/src/config.rs
//! https://raw.githubusercontent.com/rustdesk/hbb_common/<rev>/src/config.rs`. Record the two
//! revisions in the overlay's `[source]` table when bumping them.

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub from: PathBuf,
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
        let from = from.ok_or(XtaskError::MissingArgument("--from <dir>"))?;
        Ok(Self { from, check })
    }
}

pub fn run(root: &Path, args: &Args) -> Result<(), XtaskError> {
    let generated = generate(&args.from, &read(&root.join(OVERLAY))?)?;
    let path = root.join(GENERATED);
    let current = match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(source) => return Err(io_error(&path, source)),
    };
    if current.as_deref() != Some(generated.as_str()) {
        if args.check {
            return Err(XtaskError::CatalogDrift(GENERATED.to_owned()));
        }
        fs::write(&path, &generated).map_err(|source| io_error(&path, source))?;
    }
    eprintln!("xtask: {GENERATED} is up to date");
    Ok(())
}

/// The `generated.rs` text for the sources in `from` and the overlay text.
fn generate(from: &Path, overlay: &str) -> Result<String, XtaskError> {
    let config_rs = read(&from.join(source::CONFIG_RS))?;
    let keys_rs = read(&from.join(source::KEYS_RS))?;
    let keys = source::settings_keys(&config_rs, &keys_rs)
        .map_err(|e| XtaskError::Catalog(e.to_string()))?;
    let catalog = overlay::merge(&keys, overlay).map_err(|e| XtaskError::Catalog(e.to_string()))?;
    Ok(render::render(&catalog))
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
                from: PathBuf::from("src"),
                check: true
            }
        );
        assert!(!Args::parse(&args(&["--from", "x"])).unwrap().check);
        assert!(matches!(
            Args::parse(&args(&["--check"])),
            Err(XtaskError::MissingArgument(_))
        ));
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
        let overlay = "[source]\nrustdesk = \"r\"\nhbb_common = \"h\"\n[options]\n";

        let out = generate(&fixture(), overlay).unwrap();

        assert!(out.contains("key: \"enable-keyboard\""), "{out}");
        assert!(out.contains("key: \"disable-udp\""));
        assert!(
            !out.contains("\"theme\""),
            "local settings are not strategy options"
        );
        assert!(matches!(
            generate(&fixture().join("nope"), overlay),
            Err(XtaskError::Io { .. })
        ));
        assert!(matches!(
            generate(&fixture(), "[source]"),
            Err(XtaskError::Catalog(_))
        ));
    }

    #[test]
    fn check_detects_drift_and_write_fixes_it() {
        let root = tempfile::tempdir().unwrap();
        let overlay = root.path().join(OVERLAY);
        fs::create_dir_all(overlay.parent().unwrap()).unwrap();
        fs::write(
            &overlay,
            "[source]\nrustdesk = \"r\"\nhbb_common = \"h\"\n[options]\n",
        )
        .unwrap();
        let check = Args {
            from: fixture(),
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
    fn committed_catalog_matches_its_overlay_location() {
        let root = crate::repo_root();
        assert!(root.join(OVERLAY).is_file());
        assert!(root.join(GENERATED).is_file());
    }
}
