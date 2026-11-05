//! relay-key-guard: entrypoint of the deskhaven-relay image. Starts the
//! upstream relay (hbbr, unmodified) only with deskhaven's public key in
//! `KEY`, by replacing itself with hbbr (`exec`), so hbbr is the only process.
//!
//! hbbr fails open on bad keys: an empty key, or `-` / `_` when it cannot
//! write a key pair (read-only root), lets every client relay; a private key
//! is silently reduced to its public half, so the private key sat in the
//! relay's environment. Hence: `KEY` must be the base64 of a 32-byte public
//! key (what `deskhaven key show` prints), `-k` / `--key` arguments are
//! refused, and so is a `.env` in the working directory (hbbr loads it after
//! its arguments, and `KEY=` there would reopen the relay): the checked
//! variable is the only way in.
//!
//! Single file, std only: built and tested with plain `rustc` in
//! relay.Dockerfile.

use std::ffi::OsString;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode};

const HBBR: &str = "/usr/local/bin/hbbr";
const KEY_VAR: &str = "KEY";

#[derive(Debug, PartialEq, Eq)]
enum Refusal {
    Missing,
    NotPublicKey,
    KeyArgument,
    EnvFile,
}

impl Refusal {
    /// Never includes the value: it may be a private key.
    fn message(&self) -> &'static str {
        match self {
            Self::Missing => {
                "KEY is not set; set it to deskhaven's public key (`deskhaven key show`)"
            }
            Self::NotPublicKey => {
                "KEY is not a public key (44 base64 characters, as `deskhaven key show` prints); \
                 never the private key, never \"-\" or \"_\""
            }
            Self::KeyArgument => "pass the key in KEY, not as -k / --key",
            Self::EnvFile => {
                "the working directory has a .env entry; hbbr would load it and could \
                 override KEY, remove it"
            }
        }
    }
}

/// hbbr reads this from its working directory after parsing its arguments.
const ENV_FILE: &str = ".env";

/// No `.env` entry of any kind in `dir`. `symlink_metadata` does not follow
/// links, so a dangling link counts; anything but "not found" (unreadable
/// directory included) is refused too.
fn check_workdir(dir: &std::path::Path) -> Result<(), Refusal> {
    match std::fs::symlink_metadata(dir.join(ENV_FILE)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(Refusal::EnvFile),
    }
}

/// 32 bytes in padded standard base64: 43 characters and one `=`.
const PUBLIC_KEY_LEN: usize = 44;
const BASE64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
/// The last of the 43 characters carries 4 data bits; its low 2 bits are 0.
const PAD_BITS_MASK: usize = 0b11;

fn base64_value(c: u8) -> Option<usize> {
    BASE64.iter().position(|&b| b == c)
}

/// Canonical padded base64 of exactly 32 bytes; empty counts as unset.
fn check_key(key: Option<&str>) -> Result<(), Refusal> {
    let key = key.filter(|k| !k.is_empty()).ok_or(Refusal::Missing)?;
    let bytes = key.as_bytes();
    let (data, pad) = bytes
        .split_last()
        .filter(|_| bytes.len() == PUBLIC_KEY_LEN)
        .map(|(pad, data)| (data, *pad))
        .ok_or(Refusal::NotPublicKey)?;
    let values: Option<Vec<usize>> = data.iter().map(|&c| base64_value(c)).collect();
    let canonical = values
        .and_then(|v| v.last().copied())
        .is_some_and(|last| last & PAD_BITS_MASK == 0);
    if pad == b'=' && canonical {
        Ok(())
    } else {
        Err(Refusal::NotPublicKey)
    }
}

/// hbbr's key options (`-k <key>`, `-k<key>`, `--key <key>`, `--key=<key>`).
fn check_args(args: &[OsString]) -> Result<(), Refusal> {
    let is_key_option = |arg: &OsString| {
        let arg = arg.to_string_lossy();
        arg.starts_with("-k") || arg == "--key" || arg.starts_with("--key=")
    };
    if args.iter().any(is_key_option) {
        Err(Refusal::KeyArgument)
    } else {
        Ok(())
    }
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let key = std::env::var_os(KEY_VAR);
    let checked = check_workdir(std::path::Path::new("."))
        .and_then(|()| check_args(&args))
        .and_then(|()| check_key(key.as_ref().and_then(|k| k.to_str())));
    if let Err(refusal) = checked {
        let _ = writeln!(std::io::stderr(), "relay-key-guard: {}", refusal.message());
        return ExitCode::FAILURE;
    }
    let err = Command::new(HBBR).args(&args).exec();
    let _ = writeln!(
        std::io::stderr(),
        "relay-key-guard: cannot start {HBBR}: {err}"
    );
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real hbbs public key (base64 of 32 bytes).
    const PUBLIC: &str = "DHiFZnzGD91wLg4QtlwlQ7XVp9HTNMuHUEYdgO8KvUg=";

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn accepts_a_public_key() {
        assert_eq!(check_key(Some(PUBLIC)), Ok(()));
        assert_eq!(
            check_key(Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")),
            Ok(())
        );
    }

    #[test]
    fn refuses_what_hbbr_would_fail_open_on() {
        assert_eq!(check_key(None), Err(Refusal::Missing));
        assert_eq!(check_key(Some("")), Err(Refusal::Missing));
        for bad in ["-", "_", " ", "not-configured"] {
            assert_eq!(check_key(Some(bad)), Err(Refusal::NotPublicKey), "{bad}");
        }
    }

    #[test]
    fn refuses_a_private_key() {
        // hbbs id_ed25519: base64 of 64 bytes (seed + public key).
        let private = format!("{}{}", "A".repeat(43), "B".repeat(42)) + "A==";
        assert_eq!(private.len(), 88);
        assert_eq!(check_key(Some(&private)), Err(Refusal::NotPublicKey));
    }

    #[test]
    fn refuses_near_misses() {
        let unpadded = &PUBLIC[..43];
        let padded_twice = format!("{}==", &PUBLIC[..42]);
        // 43rd character with low bits set: not the encoding of 32 bytes.
        let non_canonical = format!("{}B=", &PUBLIC[..42]);
        let bad_char = format!("{}*{}", &PUBLIC[..10], &PUBLIC[11..]);
        let url_safe = format!("{}-{}", &PUBLIC[..10], &PUBLIC[11..]);
        let with_newline = format!("{PUBLIC}\n");
        for bad in [
            unpadded,
            &padded_twice,
            &non_canonical,
            &bad_char,
            &url_safe,
            &with_newline,
        ] {
            assert_eq!(check_key(Some(bad)), Err(Refusal::NotPublicKey), "{bad:?}");
        }
    }

    #[test]
    fn refuses_key_arguments_and_passes_others() {
        assert_eq!(check_args(&args(&["-p", "21117"])), Ok(()));
        assert_eq!(check_args(&args(&[])), Ok(()));
        for bad in [
            &["-k", PUBLIC][..],
            &["--key", "-"],
            &["--key=_"],
            &["-k_"],
            &["-p", "21117", "-k", "-"],
        ] {
            assert_eq!(check_args(&args(bad)), Err(Refusal::KeyArgument), "{bad:?}");
        }
    }

    /// A fresh, empty directory under the system temp dir.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("relay-key-guard-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_workdir_without_dotenv_passes() {
        let dir = scratch("clean");
        std::fs::write(dir.join("blacklist.txt"), "").unwrap();

        assert_eq!(check_workdir(&dir), Ok(()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// hbbr loads `.env` after its arguments and could reset KEY to empty.
    #[test]
    fn any_dotenv_entry_in_the_workdir_is_refused() {
        let file = scratch("file");
        std::fs::write(file.join(ENV_FILE), "KEY=\n").unwrap();
        let empty = scratch("empty");
        std::fs::write(empty.join(ENV_FILE), "").unwrap();
        let dangling = scratch("dangling");
        std::os::unix::fs::symlink("/nonexistent/target", dangling.join(ENV_FILE)).unwrap();
        let directory = scratch("directory");
        std::fs::create_dir(directory.join(ENV_FILE)).unwrap();

        for dir in [file, empty, dangling, directory] {
            assert_eq!(
                check_workdir(&dir),
                Err(Refusal::EnvFile),
                "{}",
                dir.display()
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn messages_never_echo_the_value() {
        for refusal in [
            Refusal::Missing,
            Refusal::NotPublicKey,
            Refusal::KeyArgument,
            Refusal::EnvFile,
        ] {
            assert!(!refusal.message().contains(PUBLIC));
        }
    }
}
