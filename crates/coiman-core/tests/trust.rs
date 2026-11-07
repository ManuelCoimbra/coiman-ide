//! Restricted Mode, in its own process because the gate is global: an untrusted
//! folder's settings are ignored and nothing runs for it until it is trusted.

use coiman_core::{build::cargo, project::trust, settings};

#[test]
fn untrusted_folders_are_inert_until_trusted() {
    let base = std::env::temp_dir().join(format!("rust-code-trust-it-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let folder = base.join("repo");
    std::fs::create_dir_all(folder.join(".rust-code")).unwrap();
    std::fs::write(folder.join(".rust-code/settings.toml"), "[lsp.paths]\npython = \"evil.exe\"\n").unwrap();
    // SAFETY: the only test in this process, before any thread reads the environment.
    unsafe { std::env::set_var("APPDATA", base.join("config")) };
    unsafe { std::env::set_var("XDG_CONFIG_HOME", base.join("config")) };

    trust::enforce();
    assert!(!trust::is_trusted(&folder));
    assert!(trust::check(&folder).is_err());
    assert!(settings::load(Some(&folder)).settings.lsp.paths.is_empty());
    assert!(cargo::build_executables(&folder, &["build".to_owned()]).unwrap_err().contains("Restricted"));
    assert!(cargo::binaries(&folder).is_empty());

    trust::trust(&folder).unwrap();
    assert!(trust::is_trusted(&folder.join("sub")));
    assert_eq!(settings::load(Some(&folder)).settings.lsp.paths["python"], "evil.exe");

    // Remembered, and the project can't change that: the list is in the config folder.
    trust::enforce();
    assert!(trust::is_trusted(&folder));

    trust::revoke(&folder).unwrap();
    assert!(!trust::is_trusted(&folder));
    assert!(settings::load(Some(&folder)).settings.lsp.paths.is_empty());
    trust::enforce();
    assert!(!trust::is_trusted(&folder));
    let _ = std::fs::remove_dir_all(&base);
}
