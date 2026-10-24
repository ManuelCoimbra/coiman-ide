# Rust Technology Stack and Engineering Constraints for a VS Code-style Editor on egui/eframe

Scope note: The project currently uses eframe 0.37, egui_extras with syntect highlighting, and egui's built-in `TextEdit`. Research date is 2026-10-24. Web search coverage for some crates was thin, so several items are flagged in Gaps as "not verified this session". Do not present those items as sourced facts.

## 1. Limits of egui TextEdit for code editing, and egui-based editor examples

### Takeaway
egui `TextEdit` is a general-purpose widget. It has a single cursor range and no line virtualization. It still re-lays out and tessellates a whole galley, although per-paragraph layout caching (2025) cut the cost of large edits. egui's own README admits that immediate mode makes layout hard and that very large scroll areas are slow. A serious code editor on egui should own a custom editor widget: rope buffer, visible-lines-only layout and painting, multi-cursor model, and its own undo. Use egui for the chrome (panels, tabs, palette, settings) around that widget.

### Cited Findings
- egui README: "The main disadvantage of immediate mode is it makes layout more difficult." It also says "Since an immediate mode GUI does a full layout each frame, the layout code needs to be quick" and "having a very large UI in a scroll area (with very long scrollback) can be slow." — [egui README](https://github.com/emilk/egui/blob/main/README.md)
- egui README says egui "lacks many features and the interfaces are still in flux". It also says new releases will have breaking changes. — [egui README](https://github.com/emilk/egui/blob/main/README.md)
- Issue #3086 covers large-file editing. With about 15M characters, inserting one character took more than 1 s in release mode. One profiled insert took about 2 s: about 0.5 s building glyphs, about 0.5 s in `rows_from_paragraphs`, and about 1 s tessellating rows. The cause was that `TextEdit` lays out and renders all text, not only the visible part. — [egui #3086](https://github.com/emilk/egui/issues/3086)
- emilk explained that epaint caches layout only for a whole `LayoutJob`, so any edit re-lays out the whole string. The fix caches each paragraph (split on `\n`). It does not help text with no newlines, and splitting is still O(N). The fix shipped as "Optimize editing long text by caching each paragraph" in PR #5411 (Apr 2025), which closed the issue. — [egui #3086](https://github.com/emilk/egui/issues/3086)
- The issue reporter suggested culling to visible text, as `ScrollArea::show_rows` does. Variable-width fonts make this hard, and monospace removes the dependency on earlier text. This culling is not part of the shipped fix. — [egui #3086](https://github.com/emilk/egui/issues/3086)
- The cursor API was consolidated into a single `CCursorRange` model, which models one selection range. — [egui PR #5785](https://github.com/emilk/egui/pull/5785)
- Regression in 0.37: clicking a button outside a focused `TextEdit` collapses its selection before the button's code runs. This matters for toolbar or command actions that act on a selection. — [egui #8763](https://github.com/emilk/egui/issues/8763)
- Other `TextEdit` reports: erratic selection, and wrong typing with IBus on Wayland (an IME problem). — [egui #5796](https://github.com/emilk/egui/issues/5796); [egui #7485](https://github.com/emilk/egui/issues/7485)
- Syntax colouring in `TextEdit` uses a `layouter` or colour-map approach. This dates back to a code-editor user's request. — [egui #173](https://github.com/emilk/egui/issues/173)
- Programmatic cursor and selection control is possible through `TextEdit` state. A maintainer added an example for an S-expression editor. — [egui discussion #1365](https://github.com/emilk/egui/discussions/1365)
- egui_code_editor 0.4.2 (2026-10-07) depends on egui ^0.36, which is one minor version behind the project's 0.37. Its features are line numbers, simple keyword-based highlighting, auto-completion from a syntax dictionary, auto-indent and Helix-based themes. Only 28.57% of its items are documented. — [docs.rs egui_code_editor](https://docs.rs/crate/egui_code_editor/latest)
- Zed built its own framework, GPUI, described as "a hybrid immediate and retained mode, GPU accelerated, UI framework". GPUI's low-level Elements give full layout and paint control "for large lists or custom code-editor layouts". GPUI is pre-1.0 with frequent breaking changes. On Windows it uses Win32 and DirectWrite. — [GPUI README](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md)
- A quote from the Zed team, cited in HN discussion of Zed's GPU rendering post: building their own UI framework was the only way to reach the performance they needed. A commenter noted that egui does not do the advanced GPU 2D rendering Zed described. — [HN via brianlovin.com](https://brianlovin.com/hn/35078464)
- Lapce uses Floem, its own cross-platform GPU-accelerated GUI toolkit, not egui. — [Lapce site](https://lap.dev/lapce/)

### Inferences
- `TextEdit` is fine for small files and configuration fields. For code buffers it lacks multi-cursor, column selection, folding, inlay hints, diagnostics squiggles, a gutter integrated with layout, and virtualization. Replace it with a custom widget built on `ui.allocate_rect` / `Painter` that lays out only visible lines from a rope and caches galleys per line keyed by a content hash. Use a monospace grid for O(1) x/column mapping.
- A custom widget must re-implement what `TextEdit` gives for free: IME composition (`egui::Event::Ime`), clipboard, AccessKit text-node exposure, and keyboard navigation. This is the main hidden cost.
- The 0.37 selection regression and the IME bugs show that even egui's own text widget has edge-case churn. Pin egui versions and test before each upgrade.
- Neither Zed nor Lapce chose egui. Both built retained or hybrid GPU frameworks for the editor surface. This supports keeping egui for app chrome and making the editor viewport custom-painted.

### Gaps
- No direct egui issue requesting multiple cursors in `TextEdit` was found. Single-cursor status is inferred from the `CCursorRange` model.
- `TextEdit` undo behaviour was not verified this session. From prior knowledge, egui uses an `Undoer` with time-based grouping stored in widget state, which does not suit editor-grade undo trees. Verify in egui source.
- No primary source was found for Rerun's experience with large-text or code editing in egui.
- egui_code_editor's internals (whether it wraps `TextEdit`) were not confirmed.

## 2. Text buffer data structures: ropey, crop, xi-rope, Zed SumTree

### Takeaway
Use a rope. crop is faster on real edit traces and indexes by UTF-8 bytes, like `String`, tree-sitter and LSP byte offsets. Ropey indexes by `char` and supports all Unicode line breaks. Zed's SumTree generalizes a B+ tree with pluggable summaries and gives O(log N) multi-coordinate seeks and cheap `Arc` snapshots. It is the gold standard but is not a standalone published crate. Both crop and Ropey support O(1) clone for background snapshots.

### Cited Findings
- crop's README benchmark uses crdt-benchmarks editing traces on a 2018 MacBook Pro. On automerge-paper: crop 12.39 ms, Jumprope 12.52 ms, Ropey 44.14 ms. On sveltecomponent: Ropey 3.65 ms vs crop about 0.95 ms. This benchmark is self-reported by crop's author. — [crop docs.rs](https://docs.rs/crate/crop/latest)
- crop uses UTF-8 byte offsets and recognizes only LF and CRLF line terminators. Ropey indexes by code point and can recognize all Unicode line breaks. Both support O(1) clone through shared structure. — [crop docs.rs](https://docs.rs/crate/crop/latest)
- crop's latest version is 0.4.3. — [crop docs.rs](https://docs.rs/crate/crop/latest)
- Zed's `Rope` is a `SumTree` of small inline-string `Chunk`s. A SumTree is a B+ tree where nodes store summaries, which can be any type that supports addition. `TextSummary` tracks bytes, lines, longest line and UTF-16 data, so seek and convert across offset, point and UTF-16 run in O(log N). — [Zed blog: Rope & SumTree](https://zed.dev/blog/zed-decoded-rope-sumtree)
- Zed nodes are `Arc`'d, so a snapshot only bumps reference counts. Zed uses this to re-parse with tree-sitter on a background thread after each edit. Over 20 Zed subsystems use SumTree, including diagnostics, git blame and DisplayMap. — [Zed blog: Rope & SumTree](https://zed.dev/blog/zed-decoded-rope-sumtree)
- Zed chose ropes over gap buffers and piece tables for good all-round performance, including non-local edits such as multi-cursor edits. — [Zed blog: Rope & SumTree](https://zed.dev/blog/zed-decoded-rope-sumtree)

### Inferences
- For egui plus tree-sitter plus LSP, crop's byte indexing removes conversion overhead with tree-sitter, which uses bytes. LSP still needs UTF-16 positions by default unless the server negotiates UTF-8 through `positionEncoding`. Neither crop nor Ropey tracks UTF-16 in its summaries like Zed does, so plan a conversion helper.
- Ropey is the conservative choice: Helix uses it, and it has wide adoption. crop is the faster choice. The rope crate can sit behind a small trait so it can be swapped.
- Multi-cursor edits should be applied as a batched transaction in reverse offset order, or as a Helix-style ChangeSet, against the rope.

### Gaps
- Ropey's current version and maintenance status (for example the Ropey 2.0 work) were not verified this session.
- xi-rope and Lapce's fork (lapce-xi-rope) were not verified. The Lapce site does not mention its buffer structure. xi-editor itself is archived (prior knowledge, not verified).
- No independent third-party rope benchmark was found.

## 3. Syntax highlighting: tree-sitter vs syntect

### Takeaway
syntect, the current choice, is a mature, mostly-complete TextMate/Sublime regex highlighter. It is line-state based and relatively slow on big files. tree-sitter 0.27.1 (Oct 2026) gives incremental reparse through `tree.edit()` plus a reparse, and gives real syntax trees for folding, selection expansion, outline and textobjects. Grammars can also be loaded as WASM through the optional `wasm` feature, which is how Zed ships grammars inside extensions. The recommended path is tree-sitter for supported languages with syntect as a fallback.

### Cited Findings
- tree-sitter Rust crate latest is 0.27.1, released 2026-10-08. Incremental parsing works by calling `tree.edit(InputEdit)` and then `parser.parse(text, Some(&old_tree))`, which makes parsing "much faster". — [docs.rs tree-sitter](https://docs.rs/crate/tree-sitter/latest)
- The optional `wasm` feature builds on wasmtime-c-api and is required to load WASM grammar files. — [docs.rs tree-sitter](https://docs.rs/crate/tree-sitter/latest)
- Zed extensions ship compiled tree-sitter grammars as WASM plus Scheme query files for highlighting. — [Zed blog: extensions](https://zed.dev/blog/zed-decoded-extensions)
- Zed re-parses with tree-sitter on a background thread using rope snapshots. — [Zed blog: Rope & SumTree](https://zed.dev/blog/zed-decoded-rope-sumtree)
- syntect's author describes it as mostly complete: still maintained and PRs reviewed, but not under heavy development. — [syntect GitHub](https://github.com/trishume/syntect)
- syntect benchmark on a mid-2012 MacBook Pro: jQuery 2.1 (9,200 lines, 247 KB) took about 600 ms, vs about 98 ms in Sublime Text 3. Loading default syntaxes takes about 138 ms, or about 23 ms from a precompiled dump with lazy regex compilation. — [syntect GitHub](https://github.com/trishume/syntect)
- The `fancy-regex` backend is pure Rust, which avoids Oniguruma build pain on Windows and WASM. It is about half as fast as Oniguruma. syntect exposes parse-state APIs so editors can cache and re-highlight incrementally. — [syntect GitHub](https://github.com/trishume/syntect)
- Helix advertises built-in tree-sitter, LSP, multiple selections and experimental DAP. — [Debian review of Helix](https://dfsg-new-queue.debian.org/reviews/hx)

### Inferences
- With syntect, cache the `ParseState` and `HighlightState` at line starts every N lines. After an edit, re-highlight only from the edited line until the state converges. Without this, highlighting large files in egui every frame is too slow.
- tree-sitter highlight queries (`highlights.scm`) can be reused from nvim-treesitter, Helix or Zed. Check each repository's licence before copying queries.

### Gaps
- tree-sitter grammar licensing and size were not verified this session. Most grammars are MIT, and compiled `parser.c` files can be multi-MB per language (prior knowledge). Verify per grammar before bundling.
- Current syntect version number was not confirmed (the README shows no releases section).

## 4. LSP and DAP client crates; how Helix, Zed and Lapce do it

### Takeaway
No turnkey LSP client crate exists. The standard approach is `lsp-types` for messages plus a custom JSON-RPC-over-stdio transport. async-lsp is the notable crate that supports both client and server on tower. tower-lsp and lsp-server are server-side only. The DAP crate ecosystem is weak: early-stage `dap` (server-oriented), stale `debug_types` (1.0.0, 2022), and tiny `dapts`. Expect to hand-roll the DAP client as Helix did with helix-dap.

### Cited Findings
- async-lsp is a tower-based LSP framework that "can be used to build both Language Server and Language Client". In async-lsp, requests run concurrently, notifications run in order, and handlers take `&mut self`, which avoids async locks. The repository had about 178 stars, 2 open issues and 161 commits, under Apache-2.0/MIT. — [async-lsp GitHub](https://github.com/oxalica/async-lsp)
- async-lsp's README contrasts itself with tower-lsp, which runs notifications asynchronously and offers conveniences like `Client::show_message`. It also contrasts lsp-server, which is simple, synchronous and server-only. — [async-lsp GitHub](https://github.com/oxalica/async-lsp)
- `dap` (dap-rs) is a Rust DAP implementation built around a `Server`. It is early-stage, "breakages will be frequent". `debug_types` provides DAP 1.55 types, last at 1.0.0 in 2022. `dapts` is DAP types only, at 0.0.1–0.0.6 (Mar 2025). — [docs.rs dap](https://docs.rs/crate/dap/latest); [debug_types](https://rust-digger.code-maven.com/crates/debug_types); [dapts](https://docs.rs/crate/dapts/latest)
- Helix has built-in LSP and experimental DAP. — [Debian review of Helix](https://dfsg-new-queue.debian.org/reviews/hx)
- Zed extensions provide language servers through `language_server_command`, which returns the binary, arguments and environment. Zed core runs the LSP client. — [Zed blog: extensions](https://zed.dev/blog/zed-decoded-extensions)
- Lapce has built-in LSP with completion, diagnostics and code actions. — [Lapce site](https://lap.dev/lapce/)

### Inferences
- Architecture: run one tokio task per language server, communicating over stdio JSON-RPC. Send results to the UI thread through channels and call `ctx.request_repaint()`. Never block the egui frame on LSP.
- Version documents per buffer and send incremental `didChange` events. Debounce completion and hover requests.
- DAP is the same JSON-RPC-style framing (Content-Length headers), so reuse the LSP transport and add DAP message types (debug_types or dapts, or handwritten serde structs).

### Gaps
- `lsp-types` current version and maintenance were not verified this session. It is used by rust-analyzer-adjacent crates, Helix and others (prior knowledge). Its pace has been slow, and some projects fork it.
- Helix's helix-lsp and helix-dap internals were not fetched.
- Zed's LSP client crate (`lsp`) internals were not fetched.

## 5. Integrated terminal: alacritty_terminal + portable-pty, Windows ConPTY

### Takeaway
alacritty_terminal 0.26.0 (Apr 2026) is the standard emulator core: a VTE parser plus grid and state. Zed and Lapce use it (prior knowledge, see Gaps). It also contains its own tty module with Unix openpty and Windows pieces. portable-pty from wezterm (0.9.0, Feb 2025) is the alternative PTY layer. Both are thinly documented. On Windows, PTY support means ConPTY, which requires Windows 10 1809 or later.

### Cited Findings
- alacritty_terminal latest is 0.26.0 (2026-04-06), a "Library for writing terminal emulators". Dependencies include `vte` 0.15, `rustix-openpty` (Unix), and `windows-sys` plus `miow` (Windows). Docs coverage is 63%, with examples on only 1 of 227 items. — [docs.rs alacritty_terminal](https://docs.rs/crate/alacritty_terminal/latest)
- The Alacritty application requires "ConPTY support (Windows 10 version 1809 or higher)". — [docs.rs alacritty_terminal](https://docs.rs/crate/alacritty_terminal/latest)
- portable-pty 0.9.0 (2025-02-11) is a "Cross platform pty interface" from the wezterm project, with runtime-selectable implementations. The previous release was 0.8.1 (Mar 2023), so releases are irregular. — [docs.rs portable-pty](https://docs.rs/crate/portable-pty/latest)
- Lapce ships a built-in terminal that opens at the workspace path. — [Lapce site](https://lap.dev/lapce/)

### Inferences
- Plan: use portable-pty or alacritty_terminal's `tty` to spawn the shell. Feed bytes into `alacritty_terminal::Term` on a reader thread. In egui, paint the grid cells (monospace glyphs plus background rects) only for visible rows, and map egui key events to escape sequences.
- egui has no terminal widget. Rendering, selection, scrollback, mouse reporting and IME are all custom work. An existing community crate such as `egui_term` may be a starting point (not verified).

### Gaps
- Whether alacritty_terminal's `tty` module directly wraps ConPTY on Windows was not confirmed from docs, although the `miow`/`windows-sys` dependencies suggest Windows support.
- Zed and Lapce use of alacritty_terminal is prior knowledge and was not verified this session.
- egui terminal crates (egui_term and others) were not researched.

## 6. Git: gitoxide (gix) vs git2-rs

### Takeaway
gix is pure Rust and fast-moving. It now lists clone, fetch, push, status, diff, merge, commit, blame (plumbing), rebase and worktree operations. However, the `gix` crate itself is still in "initial development / usable" tier, and some subsystems are very early, such as gix-blame and the placeholder gix-rebase. git2 (libgit2 bindings) is the mature, feature-complete option but adds a C dependency. Pragmatic choice: use gix for read-heavy paths (status, diff, blame, log) and keep git2 or shelling out to `git` for write operations like push, rebase and credentials.

### Cited Findings
- gix is the library entry point. The binaries "may forever be unstable". Production-grade crates are gix-lock and gix-tempfile. Stabilization candidates include gix-ref, gix-config, gix-hash and gix-glob. The `gix` crate itself is "usable" with incomplete functionality. gix-blame is "very early". Some crates such as gix-rebase and gix-fetchhead are placeholders. — [gitoxide README](https://github.com/GitoxideLabs/gitoxide/blob/main/README.md)
- The listed feature set ("implemented and planned", not separated) covers clone, fetch, push, status, diff, merge (blob, tree, commit), commit with hooks, blame, rebase, checkout and reset. The README calls its git2-equivalence index "definitely not exhaustive". — [gitoxide README](https://github.com/GitoxideLabs/gitoxide/blob/main/README.md)
- Merge work, including an ORT-style tree merge, was reported complete in a project update. A parallel gix-status implementation was also discussed. — [gitoxide discussion #1351](https://github.com/Byron/gitoxide/discussions/1351)
- Adoption is still being evaluated in some projects, for example Arch Linux's buildbtw evaluating gix vs libgit2 for performance. — [Arch GitLab issue](https://gitlab.archlinux.org/archlinux/buildbtw/-/issues/151)

### Inferences
- Many editors, including Zed and VS Code itself, shell out to the `git` CLI for mutating operations. That avoids credential and SSH-agent complexity. This is prior knowledge for Zed and VS Code, not verified.

### Gaps
- Current git2 version and libgit2 version were not verified this session.
- No 2026-specific gix release notes were fetched. Check `crate-status.md` before deciding.

## 7. Plugin and extension systems

### Takeaway
WASM via wasmtime with a WIT/Component-Model API (Zed's model) is the strongest option for sandboxed, language-agnostic extensions. Its cost is a deliberately narrow API: Zed extensions cannot add UI panels. Lapce uses WASI plugins. Helix's Steel (Scheme) plugin system was still unmerged as of March 2026. VS Code extension compatibility is possible only by embedding a Node-based extension host that implements the huge `vscode` API. Theia does this using Open VSX, but even Theia historically lacked 100% API coverage, and Microsoft's marketplace is off-limits to non-Microsoft products.

### Cited Findings
- Zed extensions are written in Rust, compiled to WASM and run in Wasmtime. Types are defined in WIT. Extensions use `wit_bindgen` and the host uses `wasmtime::component::bindgen!`. Each instance gets a WASI context built from the manifest. Host-to-extension calls are async, so a blocking extension does not stall the host. — [Zed blog: extensions](https://zed.dev/blog/zed-decoded-extensions)
- Zed extension capabilities are languages (tree-sitter WASM grammars plus queries), language servers, themes, snippets and slash commands. There is no support for new UI panels, arbitrary HTTP or unrestricted filesystem access. The git-based registry "doesn't scale". — [Zed blog: extensions](https://zed.dev/blog/zed-decoded-extensions)
- Lapce has a WASI-based plugin system, and plugins can be written in any language that compiles to WASI. — [Lapce site](https://lap.dev/lapce/)
- Helix's Steel plugin system is a draft PR (#8675) that depends on an event-system PR and tracks Steel master. As of March 2026 it was not merged into mainline. Forks such as helix-steel / steelix exist. — [Helix PR #8675](https://github.com/helix-editor/helix/pull/8675); [Arch wiki mirror](https://wikipedia.jakami.de/content/archlinux_en_all_maxi_2026-04/Helix); [AUR helix-steel-git](https://aur.archlinux.org/packages/helix-steel-git)
- Helix historically omitted a plugin system and implemented features in core instead. — [Debian review](https://dfsg-new-queue.debian.org/reviews/hx)
- Theia provides the VS Code extension API with extensions in a separate process and uses Open VSX by default. It is EPL-2.0 with no licence fees. A 2019 Che reply admitted "we do not have 100% API coverage", and Theia publishes an API coverage report. — [Theia FAQ](https://theia-ide.org/docs/faq/); [Theia extensions](https://theia-ide.org/docs/extensions); [Che mailing list](https://www.eclipse.org/lists/che-dev/msg03159.html)
- Microsoft does not allow access to the VS Code Marketplace from outside Visual Studio products. VS Code extensions can only access the extension API. — [EclipseSource 2021](https://eclipsesource.com/blogs/2021/03/24/vs-code-extensions-vs-theia-extensions)

### Inferences
- VS Code compatibility from a Rust/egui host would require bundling Node.js and implementing the `vscode` API surface: webviews, tree views, decorations, commands, and the many contribution points. Webviews alone imply embedding a browser engine, which egui lacks. This is a multi-year effort and not realistic for a small project. The cheaper path is to reuse VS Code ecosystem assets that are pure data: TextMate grammars (syntect can load some), themes (JSON), snippets, and language servers and debug adapters, which are standalone processes.
- Sandboxing tradeoff:
  - WASM gives capability-based isolation and deterministic API boundaries but limits plugin power.
  - Lua (mlua) or Steel embedded in-process gives full power and fast iteration, with no real sandbox unless the API is restricted.
  - Extism wraps wasmtime with a simpler host-function model, but has less type richness than WIT.

### Gaps
- Current wasmtime, extism and mlua versions and maintenance were not verified this session.
- Lapce's WASI plugin API scope (e.g. whether UI is allowed) was not verified.
- No benchmark of extension-host overhead was found.

## 8. Accessibility in egui (AccessKit)

### Takeaway
egui integrates AccessKit. Its README says native accessibility APIs are implemented on Windows and macOS, while other platforms, including web, rely on an experimental built-in screen reader. Text editing support for NVDA on Windows was developed early, with reported caret and braille-routing bugs. A custom code-editor widget must emit its own AccessKit text nodes, or screen readers will see nothing.

### Cited Findings
- egui README: egui "currently implements the native accessibility APIs on Windows and macOS". Other platforms, including web, have only an experimental built-in screen reader. — [egui README](https://github.com/emilk/egui/blob/main/README.md)
- AccessKit PR #145 targeted Windows text editing. Reviewers reported egui examples working, but noted that word navigation put the caret at the end of the current word and that braille routing buttons could not move the cursor. Rich text formatting was also missing. A later comment said AccessKit was merged into egui and that macOS text editing was coming. — [AccessKit PR #145](https://github.com/AccessKit/accesskit/pull/145)

### Inferences
- Replacing `TextEdit` with a custom widget loses egui's AccessKit text-run support unless it is re-implemented. Budget for this, or keep a hidden or alternate `TextEdit`-backed accessible view.
- The egui README line may lag reality on Linux, where AccessKit has a Unix adapter (prior knowledge, not verified).

### Gaps
- Current (0.37) state of egui and AccessKit text editing with NVDA, JAWS, Narrator or VoiceOver was not confirmed. Sources found are from about 2021–2023.

## 9. File watching, fuzzy matching, project search

### Takeaway
nucleo (the Helix project) is the best-in-class fuzzy matcher. Its `nucleo-matcher` core is described as finished. For file watching and search, notify and the ripgrep crates (ignore, grep-searcher, grep-regex) are the de facto standards, but they were not verified this session.

### Cited Findings
- nucleo's README says it is roughly 6x faster than skim (about 2.1–9.5 ms vs 16.9–35.5 ms per pattern on Linux kernel sources) and much faster than fzf with about 3M items. It uses fzf-style scoring with a more faithful Smith-Waterman affine-gap algorithm, matches graphemes, and is used by Helix. — [nucleo GitHub](https://github.com/helix-editor/nucleo)
- `nucleo-matcher` is "finished and ready for wide use", with rare breaking changes. The high-level `nucleo` crate works but lacks docs and may change API. — [nucleo GitHub](https://github.com/helix-editor/nucleo)

### Inferences
- Use the `ignore` crate's parallel walker (gitignore-aware) to index the workspace and feed paths to nucleo for Ctrl+P. Use `grep-searcher` with `grep-regex` for find-in-files, streaming results to the UI. Use `notify`, with a debouncer, to invalidate the index and reload changed buffers. All of these are prior knowledge.

### Gaps
- notify, ignore, grep-searcher and grep-regex current versions and maintenance were not verified this session. Known issue to check: notify behaviour on Windows network drives and with large trees.
- Settings and themes crates (serde plus toml/json5, file-based layered settings) were not researched. Themes could reuse VS Code JSON themes or Helix TOML themes.
