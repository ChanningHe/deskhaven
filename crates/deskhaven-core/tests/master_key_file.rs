//! Master key resolution: bootstrap `security.master_key` or `{data_dir}/master.key`.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::Path;

use deskhaven_config::Secret;
use deskhaven_core::crypto::MasterKey;
use deskhaven_core::keys::{
    KeyError, KeyWarning, load_or_generate_master_key, master_key_path, read_master_key,
};

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

#[test]
fn configured_key_wins_and_touches_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let key = MasterKey::from_bytes(&[3; 32]);
    let configured = Secret::new(key.to_base64().as_str());

    let loaded = load_or_generate_master_key(&configured, &data_dir).unwrap();

    assert_eq!(loaded.value.key_id(), key.key_id());
    assert!(loaded.warnings.is_empty());
    assert!(!data_dir.exists());
}

#[test]
fn configured_key_with_wrong_length_is_rejected_without_echo() {
    let dir = tempfile::tempdir().unwrap();
    let value = "dG9vLXNob3J0"; // "too-short"
    let err = load_or_generate_master_key(&Secret::new(value), dir.path()).unwrap_err();

    let text = format!("{err} {err:?}");
    assert!(matches!(err, KeyError::ConfiguredMasterKey(_)), "{text}");
    assert!(!text.contains(value), "{text}");
    assert!(text.contains("security.master_key"), "{text}");
}

#[test]
fn empty_config_generates_master_key_file_in_fresh_data_dir() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("nested").join("data");

    let first = load_or_generate_master_key(&Secret::default(), &data_dir).unwrap();
    let path = master_key_path(&data_dir);

    assert!(first.warnings.is_empty(), "{:?}", first.warnings);
    let contents = std::fs::read_to_string(&path).unwrap();
    assert_eq!(contents, format!("{}\n", first.value.to_base64().as_str()));
    let again = load_or_generate_master_key(&Secret::default(), &data_dir).unwrap();
    assert_eq!(again.value.key_id(), first.value.key_id());
    // Only the key file remains: the temporary file was renamed into place.
    let names: Vec<_> = std::fs::read_dir(&data_dir).unwrap().collect();
    assert_eq!(names.len(), 1);
    #[cfg(unix)]
    {
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&data_dir), 0o700);
    }
}

#[test]
fn file_content_is_accepted_as_configured_value() {
    let dir = tempfile::tempdir().unwrap();
    let generated = load_or_generate_master_key(&Secret::default(), dir.path()).unwrap();
    let contents = std::fs::read_to_string(master_key_path(dir.path())).unwrap();

    let other = tempfile::tempdir().unwrap();
    let configured = load_or_generate_master_key(&Secret::new(contents), other.path()).unwrap();
    assert_eq!(configured.value.key_id(), generated.value.key_id());
}

#[cfg(unix)]
#[test]
fn permissive_master_key_file_warns_but_loads() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let generated = load_or_generate_master_key(&Secret::default(), dir.path()).unwrap();
    let path = master_key_path(dir.path());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

    let loaded = load_or_generate_master_key(&Secret::default(), dir.path()).unwrap();

    assert_eq!(loaded.value.key_id(), generated.value.key_id());
    assert_eq!(
        loaded.warnings,
        vec![KeyWarning::PermissiveMode {
            path: path.clone(),
            mode: 0o644
        }]
    );
    let text = loaded.warnings[0].to_string();
    assert!(
        text.contains("0644") && text.contains("master.key"),
        "{text}"
    );
    assert!(!text.contains(generated.value.to_base64().as_str()));
    // Warning only: permissions are left as the operator set them.
    assert_eq!(mode(&path), 0o644);
}

#[test]
fn malformed_master_key_file_is_an_error_without_echo() {
    let dir = tempfile::tempdir().unwrap();
    let path = master_key_path(dir.path());
    std::fs::write(&path, "c2hvcnQ=\n").unwrap();

    let err = load_or_generate_master_key(&Secret::default(), dir.path()).unwrap_err();

    let text = format!("{err} {err:?}");
    assert!(matches!(err, KeyError::MasterKeyFile { .. }), "{text}");
    assert!(!text.contains("c2hvcnQ"), "{text}");
    // The broken file is never replaced by a fresh key (that would orphan ciphertexts).
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "c2hvcnQ=\n");
}

#[test]
fn read_master_key_never_creates_anything() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");

    assert!(
        read_master_key(&Secret::default(), &data_dir)
            .unwrap()
            .is_none()
    );
    assert!(!data_dir.exists());

    let generated = load_or_generate_master_key(&Secret::default(), &data_dir).unwrap();
    let read = read_master_key(&Secret::default(), &data_dir)
        .unwrap()
        .unwrap();
    assert_eq!(read.value.key_id(), generated.value.key_id());
}

#[test]
fn master_key_path_is_in_data_dir() {
    assert_eq!(
        master_key_path(Path::new("/srv/dh")),
        Path::new("/srv/dh/master.key")
    );
}

#[test]
fn configured_key_differing_from_existing_file_warns() {
    let dir = tempfile::tempdir().unwrap();
    load_or_generate_master_key(&Secret::default(), dir.path()).unwrap();
    let configured = MasterKey::from_bytes(&[4; 32]);
    let secret = Secret::new(configured.to_base64().as_str());

    for loaded in [
        load_or_generate_master_key(&secret, dir.path()).unwrap(),
        read_master_key(&secret, dir.path()).unwrap().unwrap(),
    ] {
        assert_eq!(loaded.value.key_id(), configured.key_id());
        assert_eq!(
            loaded.warnings,
            vec![KeyWarning::ConflictingMasterKeyFile {
                path: master_key_path(dir.path())
            }]
        );
    }
}

#[test]
fn configured_key_equal_to_existing_file_does_not_warn() {
    let dir = tempfile::tempdir().unwrap();
    let generated = load_or_generate_master_key(&Secret::default(), dir.path()).unwrap();
    let secret = Secret::new(generated.value.to_base64().as_str());

    let loaded = load_or_generate_master_key(&secret, dir.path()).unwrap();
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
}

#[test]
fn oversized_master_key_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(master_key_path(dir.path()), "A".repeat(4097)).unwrap();

    let err = load_or_generate_master_key(&Secret::default(), dir.path()).unwrap_err();
    assert!(matches!(err, KeyError::TooLarge { .. }), "{err:?}");
}

#[test]
fn concurrent_first_start_agrees_on_one_key() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let ids: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                s.spawn(|| {
                    load_or_generate_master_key(&Secret::default(), &data_dir)
                        .unwrap()
                        .value
                        .key_id()
                        .to_string()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(ids.windows(2).all(|w| w[0] == w[1]), "{ids:?}");
    let on_disk = read_master_key(&Secret::default(), &data_dir)
        .unwrap()
        .unwrap();
    assert_eq!(on_disk.value.key_id().to_string(), ids[0]);
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

    #[test]
    fn symlinked_master_key_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        load_or_generate_master_key(&Secret::default(), elsewhere.path()).unwrap();
        symlink(
            master_key_path(elsewhere.path()),
            master_key_path(dir.path()),
        )
        .unwrap();

        for err in [
            load_or_generate_master_key(&Secret::default(), dir.path()).unwrap_err(),
            read_master_key(&Secret::default(), dir.path()).unwrap_err(),
        ] {
            assert!(matches!(err, KeyError::Symlink { .. }), "{err:?}");
            assert!(err.to_string().contains("symlink"), "{err}");
        }
    }

    #[test]
    fn dangling_symlink_is_refused_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.key");
        symlink(&target, master_key_path(dir.path())).unwrap();

        let err = load_or_generate_master_key(&Secret::default(), dir.path()).unwrap_err();
        assert!(matches!(err, KeyError::Symlink { .. }), "{err:?}");
        assert!(!target.exists());
    }

    #[test]
    fn directory_at_key_path_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(master_key_path(dir.path())).unwrap();

        let err = load_or_generate_master_key(&Secret::default(), dir.path()).unwrap_err();
        assert!(matches!(err, KeyError::NotRegularFile { .. }), "{err:?}");
    }

    #[test]
    fn group_writable_data_dir_warns() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o775)).unwrap();

        let generated = load_or_generate_master_key(&Secret::default(), &data_dir).unwrap();
        let read = read_master_key(&Secret::default(), &data_dir)
            .unwrap()
            .unwrap();

        let expected = vec![KeyWarning::WritableDirectory {
            path: data_dir.clone(),
            mode: 0o775,
        }];
        assert_eq!(generated.warnings, expected);
        assert_eq!(read.warnings, expected);
        // Existing directories are not modified.
        assert_eq!(mode(&data_dir), 0o775);
    }
}
