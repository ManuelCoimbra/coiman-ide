# Zed extensions: review before this can reach main

MostafaXS1's Zed extension support (sent on 2026-11-28 as `coiman-source.zip`, based on v0.8.0)
lives on this branch, `zed-extensions`, kept out of main (where releases are built) until the
points below are fixed. Paths are as of the commit that brought it in.

Coiman's rules this has to keep (see PRIVACY.md): the app opens no connections of its own and
nothing at startup; anything that contacts the network says what it contacts and asks first;
downloads are asked for; code from elsewhere runs sandboxed, with what it may do granted by the
user (permissions, approved programs, trusted folders).

## Must fix

1. **Running programs without approval.** `process.run_command`
   (`coiman-core/src/extensions/zed/wasm_host.rs`, about line 453) checks the extension's own
   list only when it has one: an extension that lists nothing may run anything. Make an empty
   list mean "nothing", and ask the user to approve each program (as Coiman's own plugins do),
   only in trusted folders. Enforce the `DownloadFile` and `NpmInstall` capabilities, which are
   declared but never checked.
2. **The extension picks its writable folder.** The work folder is `work/<id>` with `id` taken
   from the extension's `extension.toml` unchecked (`zed/mod.rs`, `work_dir`; `wasm_host.rs`
   about line 1016): `id = "../../.."` opens any folder read-write. Check the id (letters,
   digits, `-`, `_`, `.` only, no `..`) and use the same id everywhere (install folder, work
   folder, cache).
3. **URLs go to curl unchecked** (`perform_http_request`, `host_download_file`, `curl_get` in
   `registry.rs`): `file://` reads local files (and a POST can send them out), and a URL
   starting with `-` is taken as a curl option (`-K` loads a config the guest wrote). Allow
   `https://` only, and put `--` before every URL.
4. **The host's environment is handed to extensions** (`shell_env`): give them only what they
   need (PATH and the like), never tokens or secrets.
5. **Network and code without asking, at startup too.** `plugins.zed_extensions` is on by
   default; the Extensions view contacts api.zed.dev as soon as it shows; restored files start
   extensions (which may download, run npm or start programs) on the first frames. Default it
   to off, say what is contacted and ask before the first registry use, before installing,
   and before an extension first runs or starts its language server (showing the command).
6. **npm** runs packages' install scripts: pass `--ignore-scripts`, and `--` before names.
7. **No integrity checks** on registry packages, downloads or GitHub assets (the Open VSX code
   checked SHA-256). Verify what can be verified (GitHub's `digest`, published hashes).
8. **No limits on extensions**: add fuel or epoch interruption and memory limits
   (`StoreLimits`), so a looping extension can't hang a thread forever.

## Should fix

- Extension files (`config.toml`, `.scm`, grammars) are read from disk on every frame
  (`sync_plugin_languages` from `plugin_decorate`): read them once, and again only when
  extensions change.
- Grammars are compiled and extensions read on the UI thread at startup and after installs:
  do it in the background.
- tree-sitter's `wasm` feature brings a second wasmtime (48) beside ours (50): one engine to
  keep updated would be better, or load grammars through our own.
- `.zip` and `.bz2` downloads need `unzip`/`bzip2`, which Windows usually lacks.
- Asset paths from manifests (themes, snippets, icons) are joined unchecked.
- An unknown API version falls back to 0.7.0; installing removes the old folder before the
  rename succeeds (the extension can be lost).
- HTTP responses drop their headers; `fetch_stream` holds the whole body.
- PRIVACY.md doesn't mention api.zed.dev, GitHub, npm or what extensions may fetch.

## Taken away here, kept in main

This branch removes things main keeps because they only read VS Code's files and are safe:
VS Code theme, snippet and language import; the compatibility report; themes from Open VSX
(SHA-256 checked, asked first); finding CodeLLDB and TypeScript in VS Code's folders. Running
VS Code extensions is on the `vscode-extensions` branch. When this is ready, it should be
brought onto today's main beside those, not in their place.

## Good, and worth keeping

Zed's format suits Coiman (WebAssembly, themes, languages, tree-sitter grammars, snippets,
icons); the tar unpacking is safe (no `..`, no links); the block comment toggle; Zed languages'
comments and outlines.
