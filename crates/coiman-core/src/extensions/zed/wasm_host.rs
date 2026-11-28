//! Wasmtime 50 component host for Zed Wasm extensions across `zed:extension` WIT versions
//! (`v0.1.0` through `v0.8.0`).

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::path::{Component as PathComponent, Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};

use flate2::read::GzDecoder;
use serde_json::{Value, json};
use tar::Archive;
use wasmtime::component::{Component, Linker, Resource, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{FsPerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use super::manifest::ExtensionManifest;
use super::registry::InstalledExtension;

// ----- Host resource types shared across all WIT versions -------------------------------------

#[derive(Clone, Debug)]
pub struct ZedWorktree {
    pub id: u64,
    pub root: PathBuf,
}

#[derive(Clone, Debug)]
pub struct ZedProject {
    pub worktree_ids: Vec<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct ZedKeyValueStore {
    pub entries: Arc<Mutex<BTreeMap<String, String>>>,
}

#[derive(Clone, Debug)]
pub struct ZedHttpResponseStream {
    pub data: Vec<u8>,
    pub pos: usize,
}

// ----- Coiman public result types -------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZedCommand {
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZedSlashCommandOutput {
    pub text: String,
    pub sections: Vec<(usize, usize, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZedSlashCompletion {
    pub label: String,
    pub new_text: String,
    pub run_command: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZedContextServerConfig {
    pub installation_instructions: String,
    pub settings_schema: String,
    pub default_settings: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZedDapBinary {
    pub command: Option<String>,
    pub arguments: Vec<String>,
    pub envs: Vec<(String, String)>,
    pub cwd: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub timeout: Option<u64>,
    pub configuration: String,
    pub request: String,
}

// ----- Bindgen modules per Zed API epoch ------------------------------------------------------

pub mod wit_v0_8 {
    pub use super::{ZedHttpResponseStream, ZedKeyValueStore, ZedProject, ZedWorktree};
    wasmtime::component::bindgen!({
        path: "wit/zed/since_v0.8.0",
        with: {
            "worktree": ZedWorktree,
            "project": ZedProject,
            "key-value-store": ZedKeyValueStore,
            "zed:extension/http-client.http-response-stream": ZedHttpResponseStream,
        },
    });
}

pub mod wit_v0_6 {
    pub use super::{ZedKeyValueStore, ZedProject, ZedWorktree, wit_v0_8};
    wasmtime::component::bindgen!({
        path: "wit/zed/since_v0.6.0",
        with: {
            "worktree": ZedWorktree,
            "project": ZedProject,
            "key-value-store": ZedKeyValueStore,
            "zed:extension/common": wit_v0_8::zed::extension::common,
            "zed:extension/context-server": wit_v0_8::zed::extension::context_server,
            "zed:extension/http-client": wit_v0_8::zed::extension::http_client,
            "zed:extension/nodejs": wit_v0_8::zed::extension::nodejs,
            "zed:extension/process": wit_v0_8::zed::extension::process,
            "zed:extension/slash-command": wit_v0_8::zed::extension::slash_command,
        },
    });
}

pub mod wit_v0_5 {
    pub use super::{ZedKeyValueStore, ZedProject, ZedWorktree, wit_v0_6, wit_v0_8};
    wasmtime::component::bindgen!({
        path: "wit/zed/since_v0.5.0",
        with: {
            "worktree": ZedWorktree,
            "project": ZedProject,
            "key-value-store": ZedKeyValueStore,
            "zed:extension/common": wit_v0_8::zed::extension::common,
            "zed:extension/context-server": wit_v0_8::zed::extension::context_server,
            "zed:extension/http-client": wit_v0_8::zed::extension::http_client,
            "zed:extension/nodejs": wit_v0_8::zed::extension::nodejs,
            "zed:extension/process": wit_v0_8::zed::extension::process,
            "zed:extension/slash-command": wit_v0_8::zed::extension::slash_command,
            "zed:extension/github": wit_v0_6::zed::extension::github,
            "zed:extension/platform": wit_v0_6::zed::extension::platform,
            "zed:extension/lsp": wit_v0_6::zed::extension::lsp,
        },
    });
}

pub mod wit_v0_2 {
    pub use super::{ZedKeyValueStore, ZedProject, ZedWorktree, wit_v0_6, wit_v0_8};
    wasmtime::component::bindgen!({
        path: "wit/zed/since_v0.2.0",
        with: {
            "worktree": ZedWorktree,
            "project": ZedProject,
            "key-value-store": ZedKeyValueStore,
            "zed:extension/http-client": wit_v0_8::zed::extension::http_client,
            "zed:extension/nodejs": wit_v0_8::zed::extension::nodejs,
            "zed:extension/slash-command": wit_v0_8::zed::extension::slash_command,
            "zed:extension/github": wit_v0_6::zed::extension::github,
            "zed:extension/platform": wit_v0_6::zed::extension::platform,
            "zed:extension/lsp": wit_v0_6::zed::extension::lsp,
        },
    });
}

pub mod wit_v0_1 {
    pub use super::{ZedKeyValueStore, ZedWorktree, wit_v0_2, wit_v0_6, wit_v0_8};
    wasmtime::component::bindgen!({
        path: "wit/zed/since_v0.1.0",
        with: {
            "worktree": ZedWorktree,
            "key-value-store": ZedKeyValueStore,
            "zed:extension/common": wit_v0_2::zed::extension::common,
            "zed:extension/http-client": wit_v0_8::zed::extension::http_client,
            "zed:extension/nodejs": wit_v0_8::zed::extension::nodejs,
            "zed:extension/platform": wit_v0_6::zed::extension::platform,
            "zed:extension/slash-command": wit_v0_8::zed::extension::slash_command,
        },
    });
}

// ----- Store state ----------------------------------------------------------------------------

pub struct WasmState {
    pub manifest: ExtensionManifest,
    pub work_dir: PathBuf,
    pub table: ResourceTable,
    pub wasi: WasiCtx,
}

impl wasmtime::component::HasData for WasmState {
    type Data<'a> = &'a mut WasmState;
}

impl WasiView for WasmState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let mut cfg = Config::new();
        cfg.wasm_component_model(true);
        Engine::new(&cfg).expect("wasmtime component engine")
    })
}

fn ls_statuses() -> &'static Mutex<BTreeMap<String, String>> {
    static STATUSES: OnceLock<Mutex<BTreeMap<String, String>>> = OnceLock::new();
    STATUSES.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub fn language_server_status(server_id: &str) -> Option<String> {
    ls_statuses()
        .lock()
        .ok()
        .and_then(|map| map.get(server_id).cloned())
        .filter(|s| !s.is_empty())
}

// ----- Shared host interface implementations --------------------------------------------------

impl wit_v0_8::zed::extension::common::Host for WasmState {}
impl wit_v0_2::zed::extension::common::Host for WasmState {}
impl wit_v0_8::zed::extension::slash_command::Host for WasmState {}
impl wit_v0_8::zed::extension::context_server::Host for WasmState {}
impl wit_v0_8::zed::extension::lsp::Host for WasmState {}
impl wit_v0_6::zed::extension::lsp::Host for WasmState {}
impl wit_v0_1::zed::extension::lsp::Host for WasmState {}

impl wit_v0_8::zed::extension::platform::Host for WasmState {
    fn current_platform(
        &mut self,
    ) -> (
        wit_v0_8::zed::extension::platform::Os,
        wit_v0_8::zed::extension::platform::Architecture,
    ) {
        use wit_v0_8::zed::extension::platform::{Architecture, Os};
        let os = if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Linux
        };
        let arch = if cfg!(target_arch = "aarch64") { Architecture::Aarch64 } else { Architecture::X8664 };
        (os, arch)
    }
}

impl wit_v0_6::zed::extension::platform::Host for WasmState {
    fn current_platform(
        &mut self,
    ) -> (
        wit_v0_6::zed::extension::platform::Os,
        wit_v0_6::zed::extension::platform::Architecture,
    ) {
        use wit_v0_6::zed::extension::platform::{Architecture, Os};
        let os = if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Linux
        };
        let arch = if cfg!(target_arch = "aarch64") { Architecture::Aarch64 } else { Architecture::X8664 };
        (os, arch)
    }
}

impl wit_v0_8::zed::extension::http_client::Host for WasmState {
    fn fetch(
        &mut self,
        req: wit_v0_8::zed::extension::http_client::HttpRequest,
    ) -> Result<wit_v0_8::zed::extension::http_client::HttpResponse, String> {
        let (headers, body) = perform_http_request(&req)?;
        Ok(wit_v0_8::zed::extension::http_client::HttpResponse { headers, body })
    }

    fn fetch_stream(
        &mut self,
        req: wit_v0_8::zed::extension::http_client::HttpRequest,
    ) -> Result<Resource<ZedHttpResponseStream>, String> {
        let (_, body) = perform_http_request(&req)?;
        self.table
            .push(ZedHttpResponseStream { data: body, pos: 0 })
            .map_err(|e| e.to_string())
    }
}

impl wit_v0_8::zed::extension::http_client::HostHttpResponseStream for WasmState {
    fn next_chunk(&mut self, res: Resource<ZedHttpResponseStream>) -> Result<Option<Vec<u8>>, String> {
        let stream = self.table.get_mut(&res).map_err(|e| e.to_string())?;
        if stream.pos >= stream.data.len() {
            return Ok(None);
        }
        let end = (stream.pos + 16_384).min(stream.data.len());
        let chunk = stream.data[stream.pos..end].to_vec();
        stream.pos = end;
        Ok(Some(chunk))
    }

    fn drop(&mut self, res: Resource<ZedHttpResponseStream>) -> wasmtime::Result<()> {
        self.table.delete(res)?;
        Ok(())
    }
}

impl wit_v0_8::zed::extension::github::Host for WasmState {
    fn latest_github_release(
        &mut self,
        repo: String,
        opts: wit_v0_8::zed::extension::github::GithubReleaseOptions,
    ) -> Result<wit_v0_8::zed::extension::github::GithubRelease, String> {
        fetch_github_release(&repo, None, opts.require_assets, opts.pre_release)
    }

    fn github_release_by_tag_name(
        &mut self,
        repo: String,
        tag: String,
    ) -> Result<wit_v0_8::zed::extension::github::GithubRelease, String> {
        fetch_github_release(&repo, Some(&tag), false, true)
    }
}

impl wit_v0_6::zed::extension::github::Host for WasmState {
    fn latest_github_release(
        &mut self,
        repo: String,
        opts: wit_v0_6::zed::extension::github::GithubReleaseOptions,
    ) -> Result<wit_v0_6::zed::extension::github::GithubRelease, String> {
        let rel = fetch_github_release(&repo, None, opts.require_assets, opts.pre_release)?;
        Ok(wit_v0_6::zed::extension::github::GithubRelease {
            version: rel.version,
            assets: rel
                .assets
                .into_iter()
                .map(|a| wit_v0_6::zed::extension::github::GithubReleaseAsset {
                    name: a.name,
                    download_url: a.download_url,
                })
                .collect(),
        })
    }

    fn github_release_by_tag_name(
        &mut self,
        repo: String,
        tag: String,
    ) -> Result<wit_v0_6::zed::extension::github::GithubRelease, String> {
        let rel = fetch_github_release(&repo, Some(&tag), false, true)?;
        Ok(wit_v0_6::zed::extension::github::GithubRelease {
            version: rel.version,
            assets: rel
                .assets
                .into_iter()
                .map(|a| wit_v0_6::zed::extension::github::GithubReleaseAsset {
                    name: a.name,
                    download_url: a.download_url,
                })
                .collect(),
        })
    }
}

impl wit_v0_1::zed::extension::github::Host for WasmState {
    fn latest_github_release(
        &mut self,
        repo: String,
        opts: wit_v0_1::zed::extension::github::GithubReleaseOptions,
    ) -> Result<wit_v0_1::zed::extension::github::GithubRelease, String> {
        let rel = fetch_github_release(&repo, None, opts.require_assets, opts.pre_release)?;
        Ok(wit_v0_1::zed::extension::github::GithubRelease {
            version: rel.version,
            assets: rel
                .assets
                .into_iter()
                .map(|a| wit_v0_1::zed::extension::github::GithubReleaseAsset {
                    name: a.name,
                    download_url: a.download_url,
                })
                .collect(),
        })
    }

    fn github_release_by_tag_name(
        &mut self,
        repo: String,
        tag: String,
    ) -> Result<wit_v0_1::zed::extension::github::GithubRelease, String> {
        let rel = fetch_github_release(&repo, Some(&tag), false, true)?;
        Ok(wit_v0_1::zed::extension::github::GithubRelease {
            version: rel.version,
            assets: rel
                .assets
                .into_iter()
                .map(|a| wit_v0_1::zed::extension::github::GithubReleaseAsset {
                    name: a.name,
                    download_url: a.download_url,
                })
                .collect(),
        })
    }
}

impl wit_v0_8::zed::extension::nodejs::Host for WasmState {
    fn node_binary_path(&mut self) -> Result<String, String> {
        crate::language::lsp::servers::which("node")
            .map(|p| p.to_string_lossy().into_owned())
            .ok_or_else(|| "node binary not found on PATH".to_owned())
    }

    fn npm_package_latest_version(&mut self, package_name: String) -> Result<String, String> {
        let url = format!("https://registry.npmjs.org/{package_name}/latest");
        let bytes = super::registry::curl_get(&url)?;
        let val: Value = serde_json::from_slice(&bytes).map_err(|e| format!("Invalid npm registry JSON: {e}"))?;
        val.get("version")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("npm package `{package_name}` missing version"))
    }

    fn npm_package_installed_version(&mut self, package_name: String) -> Result<Option<String>, String> {
        let pkg_json = self.work_dir.join("node_modules").join(&package_name).join("package.json");
        let Ok(text) = std::fs::read_to_string(pkg_json) else {
            return Ok(None);
        };
        let val: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        Ok(val.get("version").and_then(Value::as_str).map(str::to_owned))
    }

    fn npm_install_package(&mut self, package_name: String, version: String) -> Result<(), String> {
        let npm = crate::language::lsp::servers::which("npm")
            .ok_or_else(|| "npm binary not found on PATH".to_owned())?;
        std::fs::create_dir_all(&self.work_dir).map_err(|e| e.to_string())?;
        let spec = format!("{package_name}@{version}");
        let out = Command::new(npm)
            .args(["install", "--no-save", "--prefix"])
            .arg(&self.work_dir)
            .arg(&spec)
            .output()
            .map_err(|e| format!("Failed running npm: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
        }
        Ok(())
    }
}

impl wit_v0_8::zed::extension::process::Host for WasmState {
    fn run_command(
        &mut self,
        cmd: wit_v0_8::zed::extension::process::Command,
    ) -> Result<wit_v0_8::zed::extension::process::Output, String> {
        if !self.manifest.capabilities.is_empty() && !self.manifest.allows_exec(&cmd.command, &cmd.args) {
            return Err(format!(
                "capability process:exec `{}` {:?} is not listed in extension.toml",
                cmd.command, cmd.args
            ));
        }
        let mut child = Command::new(&cmd.command);
        child.args(&cmd.args);
        for (k, v) in cmd.env {
            child.env(k, v);
        }
        let out = child.output().map_err(|e| format!("Failed running `{}`: {e}", cmd.command))?;
        Ok(wit_v0_8::zed::extension::process::Output {
            status: out.status.code(),
            stdout: out.stdout,
            stderr: out.stderr,
        })
    }
}

impl wit_v0_8::zed::extension::dap::Host for WasmState {
    fn resolve_tcp_template(
        &mut self,
        template: wit_v0_8::zed::extension::dap::TcpArgumentsTemplate,
    ) -> Result<wit_v0_8::zed::extension::dap::TcpArguments, String> {
        use wit_v0_8::zed::extension::dap::{IpAddress, TcpArguments};
        let port = match template.port {
            Some(p) => p,
            None => {
                let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
                listener.local_addr().map_err(|e| e.to_string())?.port()
            }
        };
        Ok(TcpArguments {
            port,
            host: template.host.unwrap_or(IpAddress::Ipv4((127, 0, 0, 1))),
            timeout: template.timeout,
        })
    }
}

impl wit_v0_6::zed::extension::dap::Host for WasmState {
    fn resolve_tcp_template(
        &mut self,
        template: wit_v0_6::zed::extension::dap::TcpArgumentsTemplate,
    ) -> Result<wit_v0_6::zed::extension::dap::TcpArguments, String> {
        let port = match template.port {
            Some(p) => p,
            None => {
                let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
                listener.local_addr().map_err(|e| e.to_string())?.port()
            }
        };
        Ok(wit_v0_6::zed::extension::dap::TcpArguments {
            port,
            host: template.host.unwrap_or_else(|| std::net::Ipv4Addr::LOCALHOST.to_bits()),
            timeout: template.timeout,
        })
    }
}

macro_rules! impl_resource_hosts {
    ($mod:ident) => {
        impl $mod::HostWorktree for WasmState {
            fn id(&mut self, res: Resource<ZedWorktree>) -> u64 {
                self.table.get(&res).map(|w| w.id).unwrap_or(1)
            }

            fn root_path(&mut self, res: Resource<ZedWorktree>) -> String {
                self.table
                    .get(&res)
                    .map(|w| w.root.to_string_lossy().into_owned())
                    .unwrap_or_default()
            }

            fn read_text_file(&mut self, res: Resource<ZedWorktree>, path: String) -> Result<String, String> {
                let root = self
                    .table
                    .get(&res)
                    .map(|w| w.root.clone())
                    .map_err(|e| e.to_string())?;
                let target = safe_join(&root, Path::new(&path))?;
                std::fs::read_to_string(&target).map_err(|e| format!("{}: {e}", target.display()))
            }

            fn which(&mut self, _res: Resource<ZedWorktree>, binary_name: String) -> Option<String> {
                let local_bin = self.work_dir.join("node_modules/.bin").join(&binary_name);
                if local_bin.is_file() {
                    return Some(local_bin.to_string_lossy().into_owned());
                }
                crate::language::lsp::servers::which(&binary_name)
                    .map(|p| p.to_string_lossy().into_owned())
            }

            fn shell_env(&mut self, _res: Resource<ZedWorktree>) -> Vec<(String, String)> {
                std::env::vars().collect()
            }

            fn drop(&mut self, res: Resource<ZedWorktree>) -> wasmtime::Result<()> {
                let _ = self.table.delete(res);
                Ok(())
            }
        }

        impl $mod::HostKeyValueStore for WasmState {
            fn insert(&mut self, res: Resource<ZedKeyValueStore>, key: String, value: String) -> Result<(), String> {
                let kv = self.table.get(&res).map_err(|e| e.to_string())?.clone();
                kv.entries.lock().map_err(|e| e.to_string())?.insert(key, value);
                Ok(())
            }

            fn drop(&mut self, res: Resource<ZedKeyValueStore>) -> wasmtime::Result<()> {
                let _ = self.table.delete(res);
                Ok(())
            }
        }
    };
}

macro_rules! impl_project_host {
    ($mod:ident) => {
        impl $mod::HostProject for WasmState {
            fn worktree_ids(&mut self, res: Resource<ZedProject>) -> Vec<u64> {
                self.table.get(&res).map(|p| p.worktree_ids.clone()).unwrap_or_default()
            }

            fn drop(&mut self, res: Resource<ZedProject>) -> wasmtime::Result<()> {
                let _ = self.table.delete(res);
                Ok(())
            }
        }
    };
}

impl_resource_hosts!(wit_v0_8);
impl_resource_hosts!(wit_v0_6);
impl_resource_hosts!(wit_v0_5);
impl_resource_hosts!(wit_v0_2);
impl_resource_hosts!(wit_v0_1);

impl_project_host!(wit_v0_8);
impl_project_host!(wit_v0_6);
impl_project_host!(wit_v0_5);
impl_project_host!(wit_v0_2);

impl wit_v0_8::ExtensionImports for WasmState {
    fn get_settings(
        &mut self,
        _loc: Option<wit_v0_8::SettingsLocation>,
        category: String,
        key: Option<String>,
    ) -> Result<String, String> {
        host_get_settings(&category, key.as_deref())
    }

    fn download_file(
        &mut self,
        url: String,
        file_path: String,
        file_type: wit_v0_8::DownloadedFileType,
    ) -> Result<(), String> {
        use wit_v0_8::DownloadedFileType as F;
        let kind = match file_type {
            F::Bzip2 => FileKind::Bzip2,
            F::Bzip2Tar => FileKind::Bzip2Tar,
            F::Gzip => FileKind::Gzip,
            F::GzipTar => FileKind::GzipTar,
            F::Zip => FileKind::Zip,
            F::Uncompressed => FileKind::Uncompressed,
        };
        host_download_file(&self.work_dir, &url, &file_path, kind)
    }

    fn make_file_executable(&mut self, filepath: String) -> Result<(), String> {
        host_make_executable(&self.work_dir, &filepath)
    }

    fn set_language_server_installation_status(
        &mut self,
        server_name: String,
        status: wit_v0_8::LanguageServerInstallationStatus,
    ) {
        use wit_v0_8::LanguageServerInstallationStatus as S;
        let msg = match status {
            S::None => String::new(),
            S::Downloading => "Downloading language server…".to_owned(),
            S::CheckingForUpdate => "Checking for updates…".to_owned(),
            S::Failed(e) => format!("Failed: {e}"),
        };
        host_set_ls_status(&server_name, msg);
    }
}

macro_rules! impl_legacy_extension_imports {
    ($mod:ident) => {
        impl $mod::ExtensionImports for WasmState {
            fn get_settings(
                &mut self,
                _loc: Option<$mod::SettingsLocation>,
                category: String,
                key: Option<String>,
            ) -> Result<String, String> {
                host_get_settings(&category, key.as_deref())
            }

            fn download_file(
                &mut self,
                url: String,
                file_path: String,
                file_type: $mod::DownloadedFileType,
            ) -> Result<(), String> {
                let kind = match file_type {
                    $mod::DownloadedFileType::Gzip => FileKind::Gzip,
                    $mod::DownloadedFileType::GzipTar => FileKind::GzipTar,
                    $mod::DownloadedFileType::Zip => FileKind::Zip,
                    $mod::DownloadedFileType::Uncompressed => FileKind::Uncompressed,
                };
                host_download_file(&self.work_dir, &url, &file_path, kind)
            }

            fn make_file_executable(&mut self, filepath: String) -> Result<(), String> {
                host_make_executable(&self.work_dir, &filepath)
            }

            fn set_language_server_installation_status(
                &mut self,
                server_name: String,
                status: $mod::LanguageServerInstallationStatus,
            ) {
                let msg = match status {
                    $mod::LanguageServerInstallationStatus::None => String::new(),
                    $mod::LanguageServerInstallationStatus::Downloading => {
                        "Downloading language server…".to_owned()
                    }
                    $mod::LanguageServerInstallationStatus::CheckingForUpdate => {
                        "Checking for updates…".to_owned()
                    }
                    $mod::LanguageServerInstallationStatus::Failed(e) => format!("Failed: {e}"),
                };
                host_set_ls_status(&server_name, msg);
            }
        }
    };
}

impl_legacy_extension_imports!(wit_v0_6);
impl_legacy_extension_imports!(wit_v0_5);
impl_legacy_extension_imports!(wit_v0_2);
impl_legacy_extension_imports!(wit_v0_1);

// ----- Shared host helper implementations -----------------------------------------------------

enum FileKind {
    Uncompressed,
    Gzip,
    GzipTar,
    Zip,
    Bzip2,
    Bzip2Tar,
}

fn host_get_settings(category: &str, _key: Option<&str>) -> Result<String, String> {
    let loaded = crate::settings::load(None).settings;
    match category {
        "language" => Ok(json!({
            "tab_size": loaded.editor.tab_size.max(1),
            "hard_tabs": !loaded.editor.insert_spaces,
            "preferred_line_length": loaded.editor.rulers.first().copied().unwrap_or(100),
        })
        .to_string()),
        "lsp" => Ok(json!({
            "binary": null,
            "initialization_options": null,
            "settings": null,
        })
        .to_string()),
        "context_servers" => Ok(json!({
            "command": null,
            "settings": null,
        })
        .to_string()),
        other => Err(format!("Unknown settings category: {other}")),
    }
}

fn host_set_ls_status(server: &str, status: String) {
    if let Ok(mut map) = ls_statuses().lock() {
        if status.is_empty() {
            map.remove(server);
        } else {
            map.insert(server.to_owned(), status);
        }
    }
}

fn host_make_executable(work_dir: &Path, rel_or_abs: &str) -> Result<(), String> {
    let path = safe_work_path(work_dir, Path::new(rel_or_abs))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut perms = meta.permissions();
        perms.set_mode(perms.mode() | 0o111);
        std::fs::set_permissions(&path, perms).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn host_download_file(work_dir: &Path, url: &str, file_path: &str, kind: FileKind) -> Result<(), String> {
    std::fs::create_dir_all(work_dir).map_err(|e| e.to_string())?;
    let dest = safe_work_path(work_dir, Path::new(file_path))?;
    let bytes = super::registry::curl_get(url)?;
    match kind {
        FileKind::Uncompressed => {
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            std::fs::write(&dest, bytes).map_err(|e| e.to_string())
        }
        FileKind::Gzip => {
            let mut dec = GzDecoder::new(&bytes[..]);
            let mut out = Vec::new();
            dec.read_to_end(&mut out).map_err(|e| e.to_string())?;
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            std::fs::write(&dest, out).map_err(|e| e.to_string())
        }
        FileKind::GzipTar => {
            std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
            let dec = GzDecoder::new(&bytes[..]);
            let mut archive = Archive::new(dec);
            let entries = archive.entries().map_err(|e| e.to_string())?;
            for entry in entries {
                let mut entry = entry.map_err(|e| e.to_string())?;
                let rel = entry.path().map_err(|e| e.to_string())?.into_owned();
                let target = safe_join(&dest, &rel)?;
                if entry.header().entry_type().is_dir() {
                    std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
                } else if entry.header().entry_type().is_file() {
                    if let Some(p) = target.parent() {
                        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
                    }
                    let mut out = std::fs::File::create(&target).map_err(|e| e.to_string())?;
                    std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
                    #[cfg(unix)]
                    if let Ok(mode) = entry.header().mode() {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode));
                    }
                }
            }
            Ok(())
        }
        FileKind::Zip => {
            std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
            let tmp = dest.with_extension("zip.tmp");
            std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
            let out = Command::new("unzip")
                .args(["-q", "-o"])
                .arg(&tmp)
                .arg("-d")
                .arg(&dest)
                .output();
            let _ = std::fs::remove_file(&tmp);
            match out {
                Ok(o) if o.status.success() => Ok(()),
                Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_owned()),
                Err(e) => Err(format!("Failed running unzip: {e}")),
            }
        }
        FileKind::Bzip2 => {
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            let tmp = dest.with_extension("bz2.tmp");
            std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
            let out = Command::new("bzip2").args(["-dc"]).arg(&tmp).output();
            let _ = std::fs::remove_file(&tmp);
            match out {
                Ok(o) if o.status.success() => std::fs::write(&dest, o.stdout).map_err(|e| e.to_string()),
                Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_owned()),
                Err(e) => Err(format!("Failed running bzip2: {e}")),
            }
        }
        FileKind::Bzip2Tar => {
            std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
            let tmp = dest.with_extension("tbz2.tmp");
            std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
            let out = Command::new("tar").args(["-xjf"]).arg(&tmp).arg("-C").arg(&dest).output();
            let _ = std::fs::remove_file(&tmp);
            match out {
                Ok(o) if o.status.success() => Ok(()),
                Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_owned()),
                Err(e) => Err(format!("Failed running tar: {e}")),
            }
        }
    }
}

fn safe_work_path(work_dir: &Path, requested: &Path) -> Result<PathBuf, String> {
    let rel = if let Ok(stripped) = requested.strip_prefix(work_dir) {
        stripped
    } else if requested.is_absolute() {
        return Err(format!("Path escapes extension work directory: {}", requested.display()));
    } else {
        requested
    };
    safe_join(work_dir, rel)
}

fn safe_join(root: &Path, rel: &Path) -> Result<PathBuf, String> {
    let mut clean = PathBuf::new();
    for comp in rel.components() {
        match comp {
            PathComponent::CurDir => {}
            PathComponent::Normal(s) => clean.push(s),
            PathComponent::ParentDir => {
                if !clean.pop() {
                    return Err(format!("Path traversal rejected: {}", rel.display()));
                }
            }
            PathComponent::RootDir | PathComponent::Prefix(_) => {
                return Err(format!("Absolute path rejected: {}", rel.display()));
            }
        }
    }
    Ok(root.join(clean))
}

type HttpResult = Result<(Vec<(String, String)>, Vec<u8>), String>;

fn perform_http_request(
    req: &wit_v0_8::zed::extension::http_client::HttpRequest,
) -> HttpResult {
    use wit_v0_8::zed::extension::http_client::{HttpMethod, RedirectPolicy};
    let method = match req.method {
        HttpMethod::Get => "GET",
        HttpMethod::Head => "HEAD",
        HttpMethod::Post => "POST",
        HttpMethod::Put => "PUT",
        HttpMethod::Delete => "DELETE",
        HttpMethod::Options => "OPTIONS",
        HttpMethod::Patch => "PATCH",
    };
    let mut cmd = Command::new("curl");
    cmd.args(["-sSf", "--max-time", "30", "-A", "CoimanIDE/0.8.0", "-X", method]);
    match req.redirect_policy {
        RedirectPolicy::NoFollow => {}
        RedirectPolicy::FollowLimit(n) => {
            cmd.arg("-L").args(["--max-redirs", &n.to_string()]);
        }
        RedirectPolicy::FollowAll => {
            cmd.arg("-L");
        }
    }
    for (k, v) in &req.headers {
        cmd.args(["-H", &format!("{k}: {v}")]);
    }
    if let Some(body) = &req.body {
        cmd.args(["--data-binary", "@-"]).stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        let mut child = cmd.arg(&req.url).spawn().map_err(|e| e.to_string())?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(body).map_err(|e| e.to_string())?;
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
        }
        return Ok((Vec::new(), out.stdout));
    }
    let out = cmd.arg(&req.url).output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    Ok((Vec::new(), out.stdout))
}

fn fetch_github_release(
    repo: &str,
    tag: Option<&str>,
    require_assets: bool,
    pre_release: bool,
) -> Result<wit_v0_8::zed::extension::github::GithubRelease, String> {
    use wit_v0_8::zed::extension::github::{GithubRelease, GithubReleaseAsset};
    let parse_release = |v: &Value| -> Option<GithubRelease> {
        let version = v.get("tag_name").and_then(Value::as_str)?.to_owned();
        let assets = v
            .get("assets")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| {
                        Some(GithubReleaseAsset {
                            name: a.get("name")?.as_str()?.to_owned(),
                            download_url: a.get("browser_download_url")?.as_str()?.to_owned(),
                            digest: a.get("digest").and_then(Value::as_str).map(str::to_owned),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(GithubRelease { version, assets })
    };

    if let Some(tag) = tag {
        let url = format!("https://api.github.com/repos/{repo}/releases/tags/{tag}");
        let bytes = super::registry::curl_get(&url)?;
        let val: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        return parse_release(&val).ok_or_else(|| format!("Invalid GitHub release for {repo}@{tag}"));
    }

    if !pre_release && !require_assets {
        let url = format!("https://api.github.com/repos/{repo}/releases/latest");
        let bytes = super::registry::curl_get(&url)?;
        let val: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if let Some(rel) = parse_release(&val) {
            return Ok(rel);
        }
    }

    let url = format!("https://api.github.com/repos/{repo}/releases");
    let bytes = super::registry::curl_get(&url)?;
    let list: Vec<Value> = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    for item in list {
        let is_pre = item.get("prerelease").and_then(Value::as_bool).unwrap_or(false);
        let is_draft = item.get("draft").and_then(Value::as_bool).unwrap_or(false);
        if is_draft || (!pre_release && is_pre) {
            continue;
        }
        if let Some(rel) = parse_release(&item)
            && (!require_assets || !rel.assets.is_empty())
        {
            return Ok(rel);
        }
    }
    Err(format!("No matching GitHub release found for `{repo}`"))
}

// ----- Version-dispatched component wrapper ---------------------------------------------------

enum VersionedInstance {
    V0_8(wit_v0_8::Extension),
    V0_6(wit_v0_6::Extension),
    V0_5(wit_v0_5::Extension),
    V0_2(wit_v0_2::Extension),
    V0_1(wit_v0_1::Extension),
}

struct LoadedExtension {
    store: Store<WasmState>,
    instance: VersionedInstance,
}

impl LoadedExtension {
    fn load(ext: &InstalledExtension) -> Result<Self, String> {
        let wasm_path = ext.path.join("extension.wasm");
        let bytes = std::fs::read(&wasm_path).map_err(|e| format!("{}: {e}", wasm_path.display()))?;
        let api_ver = super::manifest::read_wasm_api_version(&bytes).unwrap_or((0, 7, 0));

        let work_dir = super::work_dir(&ext.id).unwrap_or_else(|| ext.path.join(".work"));
        std::fs::create_dir_all(&work_dir).map_err(|e| e.to_string())?;
        let work_str = work_dir.to_string_lossy().replace('\\', "/");

        let mut wasi_builder = WasiCtxBuilder::new();
        wasi_builder
            .env("PWD", &work_str)
            .env("RUST_BACKTRACE", "1")
            .preopened_dir(&work_dir, ".", FsPerms::ReadWrite)
            .map_err(|e| format!("WASI preopen failed: {e}"))?
            .preopened_dir(&work_dir, &work_str, FsPerms::ReadWrite)
            .map_err(|e| format!("WASI preopen failed: {e}"))?;

        let mut store = Store::new(
            engine(),
            WasmState {
                manifest: ext.manifest.clone(),
                work_dir,
                table: ResourceTable::new(),
                wasi: wasi_builder.build(),
            },
        );
        let component = Component::from_binary(engine(), &bytes).map_err(|e| format!("Wasm compile: {e}"))?;

        let add_wasi = |linker: &mut Linker<WasmState>| {
            wasmtime_wasi::p2::add_to_linker_sync(linker).map_err(|e| e.to_string())
        };

        let instance = if api_ver >= (0, 8, 0) {
            let mut linker = Linker::new(engine());
            add_wasi(&mut linker)?;
            wit_v0_8::Extension::add_to_linker::<WasmState, WasmState>(&mut linker, |s| s)
                .map_err(|e| e.to_string())?;
            let inst = wit_v0_8::Extension::instantiate(&mut store, &component, &linker)
                .map_err(|e| format!("Wasm v0.8 instantiate: {e}"))?;
            inst.call_init_extension(&mut store).map_err(|e| format!("init_extension: {e}"))?;
            VersionedInstance::V0_8(inst)
        } else if api_ver >= (0, 6, 0) {
            let mut linker = Linker::new(engine());
            add_wasi(&mut linker)?;
            wit_v0_6::Extension::add_to_linker::<WasmState, WasmState>(&mut linker, |s| s)
                .map_err(|e| e.to_string())?;
            let inst = wit_v0_6::Extension::instantiate(&mut store, &component, &linker)
                .map_err(|e| format!("Wasm v0.6 instantiate: {e}"))?;
            inst.call_init_extension(&mut store).map_err(|e| format!("init_extension: {e}"))?;
            VersionedInstance::V0_6(inst)
        } else if api_ver >= (0, 5, 0) {
            let mut linker = Linker::new(engine());
            add_wasi(&mut linker)?;
            wit_v0_5::Extension::add_to_linker::<WasmState, WasmState>(&mut linker, |s| s)
                .map_err(|e| e.to_string())?;
            let inst = wit_v0_5::Extension::instantiate(&mut store, &component, &linker)
                .map_err(|e| format!("Wasm v0.5 instantiate: {e}"))?;
            inst.call_init_extension(&mut store).map_err(|e| format!("init_extension: {e}"))?;
            VersionedInstance::V0_5(inst)
        } else if api_ver >= (0, 2, 0) {
            let mut linker = Linker::new(engine());
            add_wasi(&mut linker)?;
            wit_v0_2::Extension::add_to_linker::<WasmState, WasmState>(&mut linker, |s| s)
                .map_err(|e| e.to_string())?;
            let inst = wit_v0_2::Extension::instantiate(&mut store, &component, &linker)
                .map_err(|e| format!("Wasm v0.2 instantiate: {e}"))?;
            inst.call_init_extension(&mut store).map_err(|e| format!("init_extension: {e}"))?;
            VersionedInstance::V0_2(inst)
        } else {
            let mut linker = Linker::new(engine());
            add_wasi(&mut linker)?;
            wit_v0_1::Extension::add_to_linker::<WasmState, WasmState>(&mut linker, |s| s)
                .map_err(|e| e.to_string())?;
            let inst = wit_v0_1::Extension::instantiate(&mut store, &component, &linker)
                .map_err(|e| format!("Wasm v0.1 instantiate: {e}"))?;
            inst.call_init_extension(&mut store).map_err(|e| format!("init_extension: {e}"))?;
            VersionedInstance::V0_1(inst)
        };

        Ok(Self { store, instance })
    }

    fn push_worktree(&mut self, root: &Path) -> Result<Resource<ZedWorktree>, String> {
        self.store
            .data_mut()
            .table
            .push(ZedWorktree { id: 1, root: root.to_path_buf() })
            .map_err(|e| e.to_string())
    }

    fn push_project(&mut self, has_worktree: bool) -> Result<Resource<ZedProject>, String> {
        let ids = if has_worktree { vec![1] } else { Vec::new() };
        self.store
            .data_mut()
            .table
            .push(ZedProject { worktree_ids: ids })
            .map_err(|e| e.to_string())
    }
}

/// Manages instantiated Zed Wasm extensions and dispatches LSP, MCP, DAP, and Slash-Command calls.
#[derive(Default)]
pub struct ZedWasmHost {
    loaded: Mutex<BTreeMap<String, Arc<Mutex<LoadedExtension>>>>,
}

impl ZedWasmHost {
    fn get_or_load(&self, ext: &InstalledExtension) -> Result<Arc<Mutex<LoadedExtension>>, String> {
        if !ext.has_wasm {
            return Err(format!("Zed extension `{}` has no extension.wasm", ext.id));
        }
        if let Ok(map) = self.loaded.lock()
            && let Some(item) = map.get(&ext.id)
        {
            return Ok(item.clone());
        }
        let instance = Arc::new(Mutex::new(LoadedExtension::load(ext)?));
        if let Ok(mut map) = self.loaded.lock() {
            map.insert(ext.id.clone(), instance.clone());
        }
        Ok(instance)
    }

    pub fn unload(&self, ext_id: &str) {
        if let Ok(mut map) = self.loaded.lock() {
            map.remove(ext_id);
        }
    }

    pub fn unload_all(&self) {
        if let Ok(mut map) = self.loaded.lock() {
            map.clear();
        }
    }

    pub fn language_server_command(
        &self,
        ext: &InstalledExtension,
        server_id: &str,
        worktree_root: &Path,
    ) -> Result<ZedCommand, String> {
        let handle = self.get_or_load(ext)?;
        let mut guard = handle.lock().map_err(|e| e.to_string())?;
        let wt = guard.push_worktree(worktree_root)?;
        let LoadedExtension { store, instance } = &mut *guard;
        let cmd = match instance {
            VersionedInstance::V0_8(e) => e.call_language_server_command(store, server_id, wt),
            VersionedInstance::V0_6(e) => e.call_language_server_command(store, server_id, wt),
            VersionedInstance::V0_5(e) => e.call_language_server_command(store, server_id, wt),
            VersionedInstance::V0_2(e) => {
                return e
                    .call_language_server_command(store, server_id, wt)
                    .map_err(|e| e.to_string())?
                    .map(|c| ZedCommand { command: c.command, args: c.args, env: c.env });
            }
            VersionedInstance::V0_1(e) => {
                return e
                    .call_language_server_command(store, server_id, wt)
                    .map_err(|e| e.to_string())?
                    .map(|c| ZedCommand { command: c.command, args: c.args, env: c.env });
            }
        }
        .map_err(|e| e.to_string())??;
        Ok(ZedCommand { command: cmd.command, args: cmd.args, env: cmd.env })
    }

    pub fn language_server_initialization_options(
        &self,
        ext: &InstalledExtension,
        server_id: &str,
        worktree_root: &Path,
    ) -> Result<Option<Value>, String> {
        let handle = self.get_or_load(ext)?;
        let mut guard = handle.lock().map_err(|e| e.to_string())?;
        let wt = guard.push_worktree(worktree_root)?;
        let LoadedExtension { store, instance } = &mut *guard;
        let json_opt = match instance {
            VersionedInstance::V0_8(e) => e.call_language_server_initialization_options(store, server_id, wt),
            VersionedInstance::V0_6(e) => e.call_language_server_initialization_options(store, server_id, wt),
            VersionedInstance::V0_5(e) => e.call_language_server_initialization_options(store, server_id, wt),
            VersionedInstance::V0_2(e) => e.call_language_server_initialization_options(store, server_id, wt),
            VersionedInstance::V0_1(e) => e.call_language_server_initialization_options(store, server_id, wt),
        }
        .map_err(|e| e.to_string())??;
        parse_json_opt(json_opt)
    }

    pub fn language_server_workspace_configuration(
        &self,
        ext: &InstalledExtension,
        server_id: &str,
        worktree_root: &Path,
    ) -> Result<Option<Value>, String> {
        let handle = self.get_or_load(ext)?;
        let mut guard = handle.lock().map_err(|e| e.to_string())?;
        let wt = guard.push_worktree(worktree_root)?;
        let LoadedExtension { store, instance } = &mut *guard;
        let json_opt = match instance {
            VersionedInstance::V0_8(e) => e.call_language_server_workspace_configuration(store, server_id, wt),
            VersionedInstance::V0_6(e) => e.call_language_server_workspace_configuration(store, server_id, wt),
            VersionedInstance::V0_5(e) => e.call_language_server_workspace_configuration(store, server_id, wt),
            VersionedInstance::V0_2(e) => e.call_language_server_workspace_configuration(store, server_id, wt),
            VersionedInstance::V0_1(e) => e.call_language_server_workspace_configuration(store, server_id, wt),
        }
        .map_err(|e| e.to_string())??;
        parse_json_opt(json_opt)
    }

    pub fn context_server_command(
        &self,
        ext: &InstalledExtension,
        server_id: &str,
        has_worktree: bool,
    ) -> Result<ZedCommand, String> {
        let handle = self.get_or_load(ext)?;
        let mut guard = handle.lock().map_err(|e| e.to_string())?;
        let project = guard.push_project(has_worktree)?;
        let LoadedExtension { store, instance } = &mut *guard;
        let cmd = match instance {
            VersionedInstance::V0_8(e) => e.call_context_server_command(store, server_id, project),
            VersionedInstance::V0_6(e) => e.call_context_server_command(store, server_id, project),
            VersionedInstance::V0_5(e) => e.call_context_server_command(store, server_id, project),
            VersionedInstance::V0_2(_) | VersionedInstance::V0_1(_) => {
                return Err("Context servers require Zed Extension API >= 0.5.0".to_owned());
            }
        }
        .map_err(|e| e.to_string())??;
        Ok(ZedCommand { command: cmd.command, args: cmd.args, env: cmd.env })
    }

    pub fn context_server_configuration(
        &self,
        ext: &InstalledExtension,
        server_id: &str,
        has_worktree: bool,
    ) -> Result<Option<ZedContextServerConfig>, String> {
        let handle = self.get_or_load(ext)?;
        let mut guard = handle.lock().map_err(|e| e.to_string())?;
        let project = guard.push_project(has_worktree)?;
        let LoadedExtension { store, instance } = &mut *guard;
        let cfg = match instance {
            VersionedInstance::V0_8(e) => e.call_context_server_configuration(store, server_id, project),
            VersionedInstance::V0_6(e) => e.call_context_server_configuration(store, server_id, project),
            VersionedInstance::V0_5(e) => e.call_context_server_configuration(store, server_id, project),
            VersionedInstance::V0_2(_) | VersionedInstance::V0_1(_) => return Ok(None),
        }
        .map_err(|e| e.to_string())??;
        Ok(cfg.map(|c| ZedContextServerConfig {
            installation_instructions: c.installation_instructions,
            settings_schema: c.settings_schema,
            default_settings: c.default_settings,
        }))
    }

    pub fn complete_slash_command_argument(
        &self,
        ext: &InstalledExtension,
        name: &str,
        description: &str,
        requires_argument: bool,
        args: &[String],
    ) -> Result<Vec<ZedSlashCompletion>, String> {
        let handle = self.get_or_load(ext)?;
        let mut guard = handle.lock().map_err(|e| e.to_string())?;
        let LoadedExtension { store, instance } = &mut *guard;
        let cmd = wit_v0_8::zed::extension::slash_command::SlashCommand {
            name: name.to_owned(),
            description: description.to_owned(),
            tooltip_text: description.to_owned(),
            requires_argument,
        };
        let list = match instance {
            VersionedInstance::V0_8(e) => e.call_complete_slash_command_argument(store, &cmd, args),
            VersionedInstance::V0_6(e) => e.call_complete_slash_command_argument(store, &cmd, args),
            VersionedInstance::V0_5(e) => e.call_complete_slash_command_argument(store, &cmd, args),
            VersionedInstance::V0_2(e) => e.call_complete_slash_command_argument(store, &cmd, args),
            VersionedInstance::V0_1(e) => e.call_complete_slash_command_argument(store, &cmd, args),
        }
        .map_err(|e| e.to_string())??;
        Ok(list
            .into_iter()
            .map(|c| ZedSlashCompletion { label: c.label, new_text: c.new_text, run_command: c.run_command })
            .collect())
    }

    pub fn run_slash_command(
        &self,
        ext: &InstalledExtension,
        name: &str,
        description: &str,
        requires_argument: bool,
        args: &[String],
        worktree_root: Option<&Path>,
    ) -> Result<ZedSlashCommandOutput, String> {
        let handle = self.get_or_load(ext)?;
        let mut guard = handle.lock().map_err(|e| e.to_string())?;
        let wt = match worktree_root {
            Some(r) => Some(guard.push_worktree(r)?),
            None => None,
        };
        let LoadedExtension { store, instance } = &mut *guard;
        let cmd = wit_v0_8::zed::extension::slash_command::SlashCommand {
            name: name.to_owned(),
            description: description.to_owned(),
            tooltip_text: description.to_owned(),
            requires_argument,
        };
        let out = match instance {
            VersionedInstance::V0_8(e) => e.call_run_slash_command(store, &cmd, args, wt),
            VersionedInstance::V0_6(e) => e.call_run_slash_command(store, &cmd, args, wt),
            VersionedInstance::V0_5(e) => e.call_run_slash_command(store, &cmd, args, wt),
            VersionedInstance::V0_2(e) => e.call_run_slash_command(store, &cmd, args, wt),
            VersionedInstance::V0_1(e) => e.call_run_slash_command(store, &cmd, args, wt),
        }
        .map_err(|e| e.to_string())??;
        Ok(ZedSlashCommandOutput {
            text: out.text,
            sections: out
                .sections
                .into_iter()
                .map(|s| (s.range.start as usize, s.range.end as usize, s.label))
                .collect(),
        })
    }

    pub fn get_dap_binary(
        &self,
        ext: &InstalledExtension,
        adapter_name: &str,
        label: &str,
        config_json: &str,
        user_installed_path: Option<&str>,
        worktree_root: &Path,
    ) -> Result<ZedDapBinary, String> {
        let handle = self.get_or_load(ext)?;
        let mut guard = handle.lock().map_err(|e| e.to_string())?;
        let wt = guard.push_worktree(worktree_root)?;
        let LoadedExtension { store, instance } = &mut *guard;
        match instance {
            VersionedInstance::V0_8(e) => {
                use wit_v0_8::zed::extension::dap::{
                    DebugTaskDefinition, IpAddress, StartDebuggingRequestArgumentsRequest,
                };
                let task = DebugTaskDefinition {
                    label: label.to_owned(),
                    adapter: adapter_name.to_owned(),
                    config: config_json.to_owned(),
                    tcp_connection: None,
                };
                let bin = e
                    .call_get_dap_binary(store, adapter_name, &task, user_installed_path, wt)
                    .map_err(|e| e.to_string())??;
                Ok(ZedDapBinary {
                    command: bin.command,
                    arguments: bin.arguments,
                    envs: bin.envs,
                    cwd: bin.cwd,
                    host: bin.connection.as_ref().map(|c| match c.host {
                        IpAddress::Ipv4((a, b, c, d)) => format!("{a}.{b}.{c}.{d}"),
                        IpAddress::Ipv6(_) => "::1".to_owned(),
                    }),
                    port: bin.connection.as_ref().map(|c| c.port),
                    timeout: bin.connection.as_ref().and_then(|c| c.timeout),
                    configuration: bin.request_args.configuration,
                    request: match bin.request_args.request {
                        StartDebuggingRequestArgumentsRequest::Launch => "launch".to_owned(),
                        StartDebuggingRequestArgumentsRequest::Attach => "attach".to_owned(),
                    },
                })
            }
            VersionedInstance::V0_6(e) => {
                use wit_v0_6::zed::extension::dap::{
                    DebugTaskDefinition, StartDebuggingRequestArgumentsRequest,
                };
                let task = DebugTaskDefinition {
                    label: label.to_owned(),
                    adapter: adapter_name.to_owned(),
                    config: config_json.to_owned(),
                    tcp_connection: None,
                };
                let bin = e
                    .call_get_dap_binary(store, adapter_name, &task, user_installed_path, wt)
                    .map_err(|e| e.to_string())??;
                Ok(ZedDapBinary {
                    command: bin.command,
                    arguments: bin.arguments,
                    envs: bin.envs,
                    cwd: bin.cwd,
                    host: bin.connection.as_ref().map(|c| std::net::Ipv4Addr::from(c.host).to_string()),
                    port: bin.connection.as_ref().map(|c| c.port),
                    timeout: bin.connection.as_ref().and_then(|c| c.timeout),
                    configuration: bin.request_args.configuration,
                    request: match bin.request_args.request {
                        StartDebuggingRequestArgumentsRequest::Launch => "launch".to_owned(),
                        StartDebuggingRequestArgumentsRequest::Attach => "attach".to_owned(),
                    },
                })
            }
            _ => Err("Debug adapters require Zed Extension API >= 0.6.0".to_owned()),
        }
    }
}

fn parse_json_opt(opt: Option<String>) -> Result<Option<Value>, String> {
    match opt {
        None => Ok(None),
        Some(text) if text.trim().is_empty() => Ok(None),
        Some(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| format!("Invalid JSON from extension: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_real_toml_extension_wasm_and_queries_command_if_present() {
        let dir = Path::new("/tmp/toml_ext");
        if dir.join("extension.wasm").is_file() {
            let ext = InstalledExtension::from_folder(dir, "test", true).unwrap();
            let id = ext.id.clone();
            let host = ZedWasmHost::default();
            // Calls `init_extension` and queries LSP command; succeeds or returns binary-not-found error.
            let _res = host.language_server_command(&ext, "taplo", dir);
            let loaded = host.loaded.lock().unwrap();
            assert!(loaded.contains_key(&id), "extension `{id}` instantiated and initialized cleanly: {_res:?}");
        }
    }
}
