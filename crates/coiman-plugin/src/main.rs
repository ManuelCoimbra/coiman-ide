//! `coiman-plugin`: the publisher's side of Coiman IDE plugins.
//!
//! ```text
//! coiman-plugin keygen <secret-key-file>
//! coiman-plugin componentize <module.wasm> <plugin.wasm>
//! coiman-plugin publish <plugin-folder> <registry-folder> --key <secret-key-file> [--changelog <text>]
//! ```
//!
//! A plugin is built for `wasm32-unknown-unknown` with `wit-bindgen`;
//! `componentize` turns that module into the component Coiman IDE loads.
//! `publish` signs the plugin folder (`plugin.toml` + `plugin.wasm`) and adds
//! it to a registry folder, which can then be served over HTTP or shared as
//! a folder.

use std::path::Path;
use std::process::ExitCode;

use coiman_core::extensions::plugin::registry;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("keygen") if args.len() == 2 => keygen(Path::new(&args[1])),
        Some("componentize") if args.len() == 3 => componentize(Path::new(&args[1]), Path::new(&args[2])),
        Some("publish") if args.len() >= 3 => publish(&args),
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("coiman-plugin: {e}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "usage:
  coiman-plugin keygen <secret-key-file>
  coiman-plugin componentize <module.wasm> <plugin.wasm>
  coiman-plugin publish <plugin-folder> <registry-folder> --key <secret-key-file> [--changelog <text>]";

fn keygen(path: &Path) -> Result<(), String> {
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| e.to_string())?;
    let (secret, public) = registry::key_pair(seed);
    std::fs::write(path, format!("{secret}\n")).map_err(|e| format!("{}: {e}", path.display()))?;
    println!("Secret key written to {} (keep it private).", path.display());
    println!("Public key: {public}");
    println!("Fingerprint: {}", registry::fingerprint(&public));
    Ok(())
}

fn componentize(module: &Path, out: &Path) -> Result<(), String> {
    let bytes = std::fs::read(module).map_err(|e| format!("{}: {e}", module.display()))?;
    let component = wit_component::ComponentEncoder::default()
        .module(&bytes)
        .and_then(|e| e.validate(true).encode())
        .map_err(|e| format!("{e:#}"))?;
    std::fs::write(out, &component).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("{} ({} KB)", out.display(), component.len() / 1024);
    Ok(())
}

fn publish(args: &[String]) -> Result<(), String> {
    let (plugin, registry_dir) = (Path::new(&args[1]), Path::new(&args[2]));
    let mut key = None;
    let mut changelog = String::new();
    let mut rest = args[3..].iter();
    while let Some(flag) = rest.next() {
        match (flag.as_str(), rest.next()) {
            ("--key", Some(v)) => key = Some(v.clone()),
            ("--changelog", Some(v)) => changelog = v.clone(),
            _ => return Err(USAGE.to_owned()),
        }
    }
    let key_file = key.ok_or("--key is required")?;
    let secret = std::fs::read_to_string(&key_file).map_err(|e| format!("{key_file}: {e}"))?;
    let entry = registry::publish(registry_dir, plugin, &secret, &changelog)?;
    println!("Published {} {} to {}", entry.id, entry.version, registry_dir.display());
    println!("Publisher key fingerprint: {}", registry::fingerprint(&entry.publisher_key));
    Ok(())
}
