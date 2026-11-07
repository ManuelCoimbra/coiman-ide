//! Puts the Coiman IDE icon into the Windows executable (what Explorer, the taskbar and
//! shortcuts show), with `windres` from MinGW or MSYS2. Without it the program still
//! builds, without the icon in the file; the window sets its own icon either way.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=../../assets/icon/coiman.ico");
    println!("cargo:rerun-if-changed=build.rs");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    let gnu = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "gnu");
    if !windows || !gnu {
        return;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let ico =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets it")).join("../../assets/icon/coiman.ico");
    let rc = out.join("coiman.rc");
    // windres reads forward slashes on Windows; backslashes would be escapes.
    let ico_path = ico.display().to_string().replace('\\', "/");
    if std::fs::write(&rc, format!("1 ICON \"{ico_path}\"\n")).is_err() {
        return;
    }
    let object = out.join("coiman-icon.o");
    let built = Command::new("windres")
        .arg("-i")
        .arg(&rc)
        .arg("-O")
        .arg("coff")
        .arg("-o")
        .arg(&object)
        .status()
        .is_ok_and(|s| s.success());
    if built {
        println!("cargo:rustc-link-arg-bins={}", object.display());
    } else {
        println!("cargo:warning=windres not found: the executable is built without its icon");
    }
}
