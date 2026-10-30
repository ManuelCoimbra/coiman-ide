# Rust developer needs from an editor/IDE, 2025-2026: gap inventory for Rust Code

Tags used throughout, against the feature list supplied in the assignment:
**[HAS]** already in Rust Code. **[PARTIAL]** foundation exists, the specific capability does not. **[MISSING]** named as absent, or not in the supplied list.
Tags reflect that supplied list only. The Rust Code source was not inspected.

Source quality note: marketplace counts were read on 2026-10-30 and are cumulative installs, not active users. JetBrains blog pages returned HTTP 403, so JetBrains data is largely missing (see Gaps).

## What do Rust developers complain about in current editors, and what do surveys say?

### Takeaway
The measured pain is waiting and resources, not missing language features: 55% wait over 10 seconds for a rebuild, a third of annotation users are blocked by annotation latency, over 35% are blocked by IDE/Cargo lock contention, and rust-analyzer can reach 12-16 GB on large workspaces. Debugging is the sharpest qualitative failure: under half use a debugger at all, and 74% of debugger users' pain centres on value display, which Rust Code has partly addressed.

### Cited Findings

**State of Rust 2025 (survey Nov-Dec 2025, published 2 Mar 2026; 7,156 responses)**
- Resource usage (compile times, storage) remains a top productivity problem; debugging fell from 2nd to 4th, a shift of about 2 percentage points — [Rust Blog](https://blog.rust-lang.org/2026/03/02/2025-State-Of-Rust-Survey-results)
- Slow compilation is a "big problem" for 27.9%, and 54.68% say it could improve but does not limit them; high disk use 22.24%; subpar debugging 19.90% — [InfoWorld](https://www.infoworld.com/article/4139528/rust-developers-have-three-big-worries-survey.html). A commenter identified as a maintainer gives 27.2% and 54.7% for compile time — [Rust-Bytes issue](https://github.com/Rust-Bytes/bytes/issues/2). The two sources conflict by 0.7 points; the official post's text gives no figure because the data is in charts.
- VS Code is the most common regular editor at 51.6% — [InfoWorld](https://www.infoworld.com/article/4139528/rust-developers-have-three-big-worries-survey.html). This is down from 56.7% in 2024 (see below).
- Zed rose sharply in preference and Helix ranked second, and editors with agentic support appear to be taking users from VS Code and IntelliJ. The text gives no percentages — [Rust Blog](https://blog.rust-lang.org/2026/03/02/2025-State-Of-Rust-Survey-results)

**State of Rust 2024 (older; published Feb 2025)**
- VS Code 56.7%, vi/vim/neovim 30.2%, RustRover 16.1%, Zed 8.9% (0.7% in 2023) — [@IT, secondary report](https://atmarkit.itmedia.co.jp/ait/articles/2503/08/news086.html). The official post confirms the VS Code decline and Zed gain — [Rust Blog](https://blog.rust-lang.org/2025/02/13/2024-State-Of-Rust-Survey-results/)

**Rust Compiler Performance Survey 2025 (published 10 Sep 2025)**
- Incremental rebuilds after small changes are the most frequently reported struggle; 55% wait more than 10 seconds for a rebuild of their most frustrating project — [Rust Blog](https://blog.rust-lang.org/2025/09/10/rust-compiler-performance-survey-2025-results)
- About 87% use inline editor annotations as their main way of seeing compiler errors, and about 33% of them call waiting for those annotations a big blocker — [Rust Blog](https://blog.rust-lang.org/2025/09/10/rust-compiler-performance-survey-2025-results)
- Over 35% consider IDE and Cargo blocking each other a big problem. The documented workaround is a separate rust-analyzer target directory, at the cost of disk — [Rust Blog](https://blog.rust-lang.org/2025/09/10/rust-compiler-performance-survey-2025-results)
- `cargo check` does not share its cache with `cargo build`, so running both compiles extra work — [Rust Blog](https://blog.rust-lang.org/2025/09/10/rust-compiler-performance-survey-2025-results)
- rust-analyzer speed and memory were frequently mentioned in open answers; PGO gave 15-20% wins in distributed builds — [Rust Blog](https://blog.rust-lang.org/2025/09/10/rust-compiler-performance-survey-2025-results)
- Only very few developers profile builds. `cargo build --timings` gives limited information, `-Zself-profile` is hard to read without compiler knowledge, and planned tooling would explain what was recompiled and which macros are slow — [Rust Blog](https://blog.rust-lang.org/2025/09/10/rust-compiler-performance-survey-2025-results)
- Three rebuild bottlenecks: unnecessary rebuilds of dependent workspace crates, slow linking, slow single-crate incremental rebuilds. Derive expansion is not cached. Switching the dev profile to `line-tables-only` showed 2-30% cycle-count improvement — [Rust Blog](https://blog.rust-lang.org/2025/09/10/rust-compiler-performance-survey-2025-results)
- Average satisfaction with build performance is about 6/10 — [Rust newsletter](https://weeklyrust.substack.com/p/compiler-survey-results-are-shocking). Respondent counts conflict: 3,700+ there versus 3,200+ in [@IT](https://atmarkit.itmedia.co.jp/ait/spv/2509/22/news014.html).

**Rust Debugging Survey 2026 (Feb 2026, published 7 Sep 2026; 2,300+ responses)**
- Over 46% currently use a debugger for Rust; print debugging and `dbg!` are the top two approaches; about 81% prefer logs or prints as easier or faster — [Rust Blog](https://blog.rust-lang.org/2026/09/07/rust-debugging-survey-2026-results/)
- Excluding prints, `lldb` inside an IDE is the most popular setup, ahead of `gdb` on the command line. On Linux the gap is only 0.4%; on Windows, WSL and macOS, IDE lldb leads by at least 6% — [Rust Blog](https://blog.rust-lang.org/2026/09/07/rust-debugging-survey-2026-results/)
- Slightly over 74% report poor value representation and just over 55% cannot print variables. Over 51% hit stepping issues: async slightly over 28%, macros about 23%, function pointers almost 6% — [Rust Blog](https://blog.rust-lang.org/2026/09/07/rust-debugging-survey-2026-results/)
- Troublesome types include enums, `HashMap` and `Vec`. Only about a quarter use debuggers on async code. 44% debug mixed-language programs: C a little over 70%, C++ about 43%, Python about 20% — [Rust Blog](https://blog.rust-lang.org/2026/09/07/rust-debugging-survey-2026-results/)
- Respondents commonly suggest displaying values through each type's `Debug` implementation, but `Debug` code may be absent from binaries unless used. BugStalker does this under that condition — [Rust Blog](https://blog.rust-lang.org/2026/09/07/rust-debugging-survey-2026-results/)
- Nearly 62% are reported as unaware of the `debugger_visualizer` attribute — [Rust Blog](https://blog.rust-lang.org/2026/09/07/rust-debugging-survey-2026-results/). The summary read here words the denominator ambiguously; [DevOps.com](https://devops.com/rusts-first-debugging-survey-shows-most-developers-skip-the-debugger-entirely/) says 62% of library authors.

**rust-analyzer resources and indexing**
- Nearly 13 GB on a workspace of about 830 dependencies (Sept 2025). A community replier calls this typical, says there is "no complete way to diagnose memory overuse problems", and the "Memory usage (clears database)" command is reported as doing nothing. `rust-analyzer analysis-stats` is offered as an approximate substitute — [Rust users forum](https://users.rust-lang.org/t/rust-analyzer-using-13-gib/133914)
- About 12 GB idle and up to 16 GB while indexing on the Zed codebase — [RA discussion #20871, via search result](https://github.com/rust-lang/rust-analyzer/discussions/20871)
- Commenters report about 2 GiB per instance, over 8 GB, about 13 GB and over 24 GB. Some cap it with cgroups so it is killed rather than starving builds; multiple workspaces multiply the cost; indexing latency recurs. Anecdotal — [HN thread on rust-glancer](https://hn.nuxt.dev/item/49393052)
- About 75,000 lines of Rust took "2-3 minutes on startup" (Mar 2025). Persistent caches (#4712, opened 2020) remain open, labelled E-hard and S-unactionable, with ra-multiplex recommended meanwhile and a separate tool, analyzed, shares a backend across sessions — [RA #4712](https://github.com/rust-lang/rust-analyzer/issues/4712)
- The performance plan (#17491, June 2024; last comment seen Feb 2026) names: Rowan tree building, quadratic `macro_rules!` expansion, a scan-vulnerable LRU, non-incremental crate graph, about 5 seconds per 100 crates for project loading, Chalk cost in autocomplete, and serial VFS loading. A prototype loaded 876 crates in about 1.5 seconds versus over a minute — [RA #17491](https://github.com/rust-lang/rust-analyzer/issues/17491). The issue is open; which items have landed was not checked.
- matklad (Aug 2026) argues rust-analyzer should not use salsa for dependencies, proposing a salsa layer for edited code plus a read-only on-disk layer based on `.rmeta`. He also says LSP cannot guarantee consistent snapshots, giving only "heuristically right" results — [matklad](https://matklad.github.io/2026/08/21/rust-glancer.html)
- rust-glancer is a new alternative LSP targeting under 100 MB, built with LLM help, keeping analysis on disk, with about 225 MB of artifacts for its own codebase and without full rust-analyzer feature coverage — [HN thread](https://hn.nuxt.dev/item/49393052); [dev.to](https://dev.to/adilaidev/cut-rust-lsp-ram-use-without-losing-core-features-3em1)
- Top open issues by thumbs-up, in order: organize imports (#5131), persistent caches (#4712), "Move item" refactoring (#2178), `rust-analyzer.toml` (#13529), faster plan (#17491), live-update diagnostics (#3107), auto semicolon (#3830), doctests as Rust code (#4170), type annotations looking through typedefs (#1666), invalid offset error (#17289) — [GitHub sorted query](https://github.com/rust-lang/rust-analyzer/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc). Reaction counts were not visible.
- A regression report describes v0.3.2786 (Mar 2026) leaving on-save diagnostics stale after edits; fix status unverified — [task description](https://hub.harborframework.com/tasks/abundant/rust-lang__rust-analyzer-21627). Changelog #311 (Jan 2026) added flycheck triggering when non-workspace files change — [RA changelog](https://rust-analyzer.github.io/thisweek/2026/01/19/changelog-311.html)

**Other editors (mostly anecdotal, 2024-2026)**
- RustRover does not use rust-analyzer — [JetBrains, via search snippet](https://blog.jetbrains.com/rust/2026/10/01/rustrover-vs-vs-code/). One user calls it a memory monster — [r/rust, Dec 2024](https://redlib.hbubli.cc/r/rust/comments/1hkfins/what_ide_for_rust_do_you_choose). Another says it does not show all errors — [r/learnrust, Apr 2025](https://redlib.hackliberty.org/r/learnrust/comments/1k1l7np/rustrover_or_text_editor)
- Zed is praised for speed, but its own guide says some RustRover refactorings may have no rust-analyzer equivalent — [Zed migration docs](https://zed.dev/docs/migrate/rustrover); [users forum](https://users.rust-lang.org/t/is-there-a-standard-recommended-ide-for-rust/137499). Zed opens macro expansion in a dedicated multibuffer — [Zed commit](https://git.secluded.site/zed/commit/cdf7c10b57b10c47823c4f0e57d473a0dde4d238)
- Helix is praised for out-of-the-box LSP, with its non-portable keybindings cited as a barrier — [r/rust thread](https://redlib.hbubli.cc/r/rust/comments/1hkfins/what_ide_for_rust_do_you_choose)

### Inferences
- "Instant, low-memory, tuned-by-default rust-analyzer" is the most evidence-backed positioning, because latency and memory are measured complaints and most fixes sit at the client. A native egui client does not reduce rust-analyzer's own memory, so claims must not imply that.
- Defaulting users to a separate rust-analyzer target directory addresses the 35% lock contention directly, with a visible disk-cost setting.
- Zed's rise and the agentic-editor shift suggest VS Code loyalty is weakening, but agentic support is now competitive table stakes that this research did not assess.

### Gaps
- Helix and Neovim exact shares for 2025, and all 2025 editor percentages except VS Code: official data is in charts not readable here.
- JetBrains Developer Ecosystem 2025 Rust numbers: [the post](https://blog.jetbrains.com/rust/2026/02/11/state-of-rust-2025/) returned 403. The same applies to the Oct 2026 RustRover vs VS Code post.
- No 2026 r/rust thread with structured complaints was retrieved; Reddit evidence here is 2024-2025 and anecdotal.
- No independent memory benchmark of rust-analyzer versus RustRover or rust-glancer was found; matklad's post gives no absolute measurements.

## Which Rust-related extensions are popular and what do they fill?

### Takeaway
Installs confirm the essentials: debugger, inline errors, rust-analyzer, TOML, and dependency version lens. Research-grade ownership tools are three orders of magnitude smaller, so they are differentiators rather than proven demand.

### Cited Findings
| Extension | Installs (2026-10-30) | Fills | Rust Code |
|---|---|---|---|
| [CodeLLDB](https://marketplace.visualstudio.com/items?itemName=vadimcn.vscode-lldb) | 11,097,601 (not Rust-only) | Native debugging | [HAS] |
| [Error Lens](https://marketplace.visualstudio.com/items?itemName=usernamehw.errorlens) | 10,206,172 (not Rust-only) | Inline diagnostics | [HAS] |
| [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer) | 6,800,463 | Language server | [HAS] |
| [Even Better TOML](https://marketplace.visualstudio.com/items?itemName=tamasfe.even-better-toml) | 5,008,917 | TOML validation, formatting, schema completion, hover | [MISSING], not in supplied list |
| [crates](https://marketplace.visualstudio.com/items?itemName=serayuzgur.crates) | 1,843,484; archived and deprecated, points to Dependi | Version lens, update all, docs.rs links | [MISSING] |
| [Dependi](https://marketplace.visualstudio.com/items?itemName=fill-labs.dependi) | 773,547; vulnerability reports and private repos are paid | Multi-language version lens | [MISSING] |
| [Tombi](https://marketplace.visualstudio.com/items?itemName=tombi-toml.tombi) | 28,402 | TOML formatter, linter, LSP | [MISSING] |
| [RustOwl](https://marketplace.visualstudio.com/items?itemName=cordx56.rustowl-vscode) | 10,080; [about 5.2k GitHub stars](https://github.com/cordx56/rustowl) | Ownership and lifetime underlines | [MISSING] |
| [Flowistry](https://marketplace.visualstudio.com/items?itemName=wcrichton.flowistry) | 7,356 | Information-flow focus mode | [MISSING] |

- Community Rust packs commonly bundle rust-analyzer, CodeLLDB, Even Better TOML and Dependi, as does Burn's contributor setup guide — [Rust Extension Pack](https://marketplace.visualstudio.com/items?itemName=notsecret32.rust-extension-pack-vscode); [Burn](https://burn.dev/contributor-book/getting-started/configuring-your-editor.html)
- cargo-appraiser is a `Cargo.toml` language server showing audit warnings, requiring the `cargo-audit` binary — [marketplace](https://marketplace.visualstudio.com/items?itemName=washan.cargo-appraiser)

### Inferences
- Cargo.toml intelligence is the largest install-backed gap: combined TOML and version-lens installs are in the millions, and Dependi paywalls vulnerability reports, leaving room for a free built-in version.
- Summed installs overlap and are cumulative, so they do not count unique users. Crates' deprecation means many of its installs may be stale.

### Gaps
- Install counts for Rust Test Explorer, rust-syntax, Prettier Rust and cargo-expand helpers were not retrieved. Aquascope has no marketplace figure here because [its main use is an mdBook preprocessor](https://github.com/cognitive-engineering-lab/aquascope/).

## Which wished-for features matter, and how could Rust Code do them natively?

### Takeaway
Evidence is strong for build-wait transparency, debugger value display, Cargo.toml tooling and ownership visualization interest. It is weak or absent for binary size, Criterion, WASM and rustdoc preview: their inclusion below reflects the assignment's list, not measured demand. The "Native idea" column is inference.

### Cited Findings

| Need | Status | Evidence | Native idea (inference) |
|---|---|---|---|
| Annotation and flycheck latency | [PARTIAL] check-on-save exists | [87% rely on annotations; 33% blocked](https://blog.rust-lang.org/2025/09/10/rust-compiler-performance-survey-2025-results) | Live status bar with elapsed time and queued state; separate target dir default; [`check.workspace=false`](https://rust-analyzer.github.io/book/configuration.html) offered on big workspaces |
| IDE/Cargo lock contention | [MISSING] | [Over 35% big problem](https://blog.rust-lang.org/2025/09/10/rust-compiler-performance-survey-2025-results) | Detect "blocking waiting for file lock", show the holder, offer one-click separate target dir |
| rust-analyzer status and memory | [MISSING] | [No complete diagnostic](https://users.rust-lang.org/t/rust-analyzer-using-13-gib/133914); [`experimental/serverStatus`, `analyzerStatus`](https://rust-analyzer.github.io/book/contributing/lsp-extensions.html) | Process RSS graph from the OS, health from serverStatus, restart and reload buttons, per-workspace budget warning |
| Sharing one analyzer across windows | [MISSING] | [ra-multiplex, analyzed, #4712](https://github.com/rust-lang/rust-analyzer/issues/4712) | Single-process multi-window design sharing one server per workspace |
| Memory-saving profiles | [MISSING] | [Options: `lru.capacity`, `cachePriming.enable`, `procMacro.processes`, `numThreads`](https://rust-analyzer.github.io/book/configuration.html) | "Low memory" and "large workspace" presets with explained tradeoffs |
| Build time explanation | [MISSING] | [`--timings` limited; few profile](https://blog.rust-lang.org/2025/09/10/rust-compiler-performance-survey-2025-results) | Native interactive Gantt from `cargo build --timings` data, plus "why rebuilt" and per-crate ranking; compare two runs |
| Debugger value display | [PARTIAL] formatters for Vec/String/Option/HashMap | [74% poor representation; enums, HashMap, Vec named](https://blog.rust-lang.org/2026/09/07/rust-debugging-survey-2026-results/) | Extend to all enums and common crates; regression-test formatters per toolchain; market this as a headline |
| Printing variables reliably | [PARTIAL] watch exists | [55% cannot print variables](https://blog.rust-lang.org/2026/09/07/rust-debugging-survey-2026-results/) | Clear "optimized out" or "no debuginfo" reason with a suggested profile fix |
| Print-debugging ergonomics | [PARTIAL] logpoints exist | [81% prefer logs and prints; `dbg!` near top](https://blog.rust-lang.org/2026/09/07/rust-debugging-survey-2026-results/) | Treat `dbg!` output as first-class: click to source, inline value, "remove all dbg!" |
| Async/tokio debugging | [MISSING] | [Async about 25% usage; 28% stepping issues](https://blog.rust-lang.org/2026/09/07/rust-debugging-survey-2026-results/) | Logical async stack traces; tasks view; the integration path needs prototyping |
| Stepping through macros and iterators | [PARTIAL] skip-std exists | [Macros 23%](https://blog.rust-lang.org/2026/09/07/rust-debugging-survey-2026-results/); [iterators and Futures named](https://blog.rust-lang.org/2026/09/07/rust-debugging-survey-2026-results/) | Skip rules for macro-generated code and iterator adapters |
| Debugger setup guidance, mixed-language | [PARTIAL] | [Docs requested; 44% mixed-language](https://blog.rust-lang.org/2026/09/07/rust-debugging-survey-2026-results/) | Zero-config debug, doctor panel for missing debuginfo, FFI frames shown across C/C++ |
| Crates.io version hints | [MISSING] | [crates 1.84M](https://marketplace.visualstudio.com/items?itemName=serayuzgur.crates), [Dependi 774k](https://marketplace.visualstudio.com/items?itemName=fill-labs.dependi) | Inline version lens, update one or all, feature list on hover, sparse-index cache |
| Cargo.toml schema and validation | [MISSING] | [Even Better TOML 5.0M](https://marketplace.visualstudio.com/items?itemName=tamasfe.even-better-toml) | Built-in schema completion; `experimental/openCargoToml` |
| Dependency audit | [MISSING] | [cargo-appraiser and Cargo-Deps need separate binaries](https://marketplace.visualstudio.com/items?itemName=washan.cargo-appraiser); [Dependi paywall](https://marketplace.visualstudio.com/items?itemName=fill-labs.dependi); [cargo-deny covers licenses, bans, sources; udeps is nightly-only](https://microsoft.github.io/RustTraining/engineering-book/ch06-dependency-management-and-supply-chain-s.html) | Advisory lens in `Cargo.toml`; deny.toml panel; installer prompts for tools |
| Macro expansion view | [MISSING] | [`rust-analyzer/expandMacro`](https://rust-analyzer.github.io/book/contributing/lsp-extensions.html); [Zed has it](https://git.secluded.site/zed/commit/cdf7c10b57b10c47823c4f0e57d473a0dde4d238) | Inline expandable block rendered as read-only overlay, rather than a separate tab |
| Inlay hints | [MISSING] | [Type, parameter, chaining and closing-brace hints on by default](https://rust-analyzer.github.io/book/configuration.html) | Standard LSP inlay support; hover-to-reveal mode; `inlayHints.maxLength` handling |
| Semantic highlighting | [MISSING] | [`semanticHighlighting.*` options](https://rust-analyzer.github.io/book/configuration.html) | Merge tokens over tree-sitter; style for unsafe, mutable and consuming usage (modifier availability to verify) |
| Memory layout viewer | [MISSING] | [`viewRecursiveMemoryLayout`; generics yield nothing](https://rust-analyzer.github.io/book/contributing/lsp-extensions.html) | Native byte-map panel showing padding and niches, an egui strength |
| Workspace and crate graph | [MISSING] | [`viewCrateGraph` SVG; `fetchDependencyList`](https://rust-analyzer.github.io/book/contributing/lsp-extensions.html) | Native interactive graph and dependency tree rather than SVG |
| Ownership and lifetime overlay | [MISSING] | [RustOwl 5.2k stars, has its own LSP](https://github.com/cordx56/rustowl); [VS Code decoration limits hurt it](https://github.com/cordx56/rustowl) | Integrate RustOwl's LSP and render precise underlines, exploiting the custom renderer |
| Information-flow focus | [MISSING] | [Flowistry: max Rust 1.73, requires nightly-2023-08-25](https://github.com/willcrichton/flowistry); [RA lacks needed MIR and borrow support](https://github.com/willcrichton/flowistry) | Do not depend on it; research only |
| Borrow-check teaching views | [MISSING] | [Aquascope mainly mdBook](https://github.com/cognitive-engineering-lab/aquascope/) | Low priority for the editor; link out |
| Embedded flash and debug | [MISSING] | [probe-rs DAP server; extension alpha; single-core; generics TODO; RTT and defmt](https://probe.rs/docs/tools/debugger/) | Reuse DAP client; add RTT panel and SVD register view |
| Hot reload | [MISSING] | [Subsecond experimental, `dx serve --hotpatch`](https://dioxuslabs.com/learn/0.7/essentials/ui/hotreload); [Bevy community crates only](https://docs.rs/crate/bevy_hotpatching_experiments/latest/source/readme.md); [Bevy and Iced integrations](https://docs.rs/subsecond/=0.7.10/); [`procMacro.ignored` guidance](https://docs.rs/crate/bevy_hotpatching_experiments/latest/source/readme.md) | Run-config template; show patch status from `dx` |
| Organize imports, doctest analysis | [MISSING] | [Top RA issues #5131, #4170](https://github.com/rust-lang/rust-analyzer/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc) | Editor-side import sorting action; doctests are server limited |
| Disk use of `target` | [MISSING] | [22.24% big problem](https://www.infoworld.com/article/4139528/rust-developers-have-three-big-worries-survey.html) | Size display, per-profile clean; warn when a separate analyzer dir doubles use |
| Test running | [HAS] | [`relatedTests`, `experimental/runTest` family](https://rust-analyzer.github.io/book/contributing/lsp-extensions.html) | Add "run related tests" from a function |
| Binary size, Criterion, coverage, Miri, flamegraph, WASM, rustdoc preview, `--explain`, Clippy fix-all, feature-flag UI | [MISSING] | No demand data retrieved; see Gaps. `cargo-bloat`: [no extension found](https://marketplace.visualstudio.com/items?itemName=washan.cargo-appraiser) in one search, a weak negative | Wrap as tasks with native result panels once demand is validated |

### Inferences
- Rust Code's clearest near-term advantage is debugging: lldb in an IDE is already the leading non-print setup on Windows, WSL and macOS, and formatters target the survey's top complaint. Coverage of arbitrary enums is the key thing to verify.
- A custom renderer can do what VS Code cannot: RustOwl's documented decoration limit, and inline macro expansion, memory layout and graphs, are natural native showcases.
- Suggested priority by evidence strength: (1) inlay hints and semantic tokens, a parity gap; (2) analyzer status, presets and lock-contention handling; (3) Cargo.toml version lens, schema and audit; (4) macro expansion; (5) timings panel; (6) debugger enum coverage and `dbg!` tooling; (7) memory layout, crate graph; (8) RustOwl overlay; (9) embedded and hot reload.

### Gaps
- No reliable quantified demand was found for cargo-bloat, Criterion, coverage, Miri, flamegraph, WASM, rustdoc preview, `rustc --explain` inline, Clippy fix-all or feature-flag UI. The search budget ended before separate queries on each. Treat them as hypotheses.
- The Flowistry nightly requirement is from its README's troubleshooting text; whether RustOwl needs nightly was not confirmed on the page read.
- Aquascope and Flowistry user-study results were not retrieved. The Flowistry paper is ["Modular Information Flow through Ownership", PLDI 2022](https://github.com/willcrichton/flowistry).

## What do rust-analyzer maintainers say clients get wrong or underuse?

### Takeaway
No direct maintainer statement listing common client mistakes was found. The extension docs still show which methods are meant for users versus debugging, and several user-facing methods map directly to Rust Code's missing features.

### Cited Findings
All from the [LSP extensions doc](https://rust-analyzer.github.io/book/contributing/lsp-extensions.html), which calls itself best effort. Capabilities are enabled through the `experimental` field.

| Method | Purpose | Note | Rust Code |
|---|---|---|---|
| `experimental/runnables` | Tests, binaries, `cargo check -p` | | [PARTIAL], use unverified |
| `discoverTest`, `discoveredTests`, `runTest`, `endRunTest`, `abortRunTest`, `changeTestState`, `appendOutputToRunTest` | Server-driven test explorer | | [PARTIAL], Rust Code's explorer implementation not inspected |
| `rust-analyzer/relatedTests` | Tests related to a position | | [MISSING] |
| `rust-analyzer/expandMacro` | Expands macro at position | | [MISSING] |
| `rust-analyzer/viewRecursiveMemoryLayout` | Type layout tree | Generics yield nothing when selected directly | [MISSING] |
| `rust-analyzer/viewCrateGraph` | Crate graph as SVG | | [MISSING] |
| `rust-analyzer/fetchDependencyList` | Crates for a dependency tree | | [MISSING] |
| `experimental/externalDocs` | Web and local docs URLs | Local URLs need `localDocs` capability | [MISSING], docs.rs bridge |
| `experimental/serverStatus` | Health and pending work | Clients may ignore it | [MISSING] |
| `rust-analyzer/analyzerStatus` | Internal status | Debugging | [MISSING] |
| `reloadWorkspace`, `rebuildProcMacros` | Reload project; rebuild macros | | [MISSING] |
| `runFlycheck`, `clearFlycheck`, `cancelFlycheck` | Control flycheck | | [MISSING], enables cancel in UI |
| `experimental/ssr` | Structural search and replace | Search-only mode and scope are open questions | [MISSING] |
| `parentModule`, `joinLines`, `matchingBrace`, `moveItem`, `onEnter`, `openCargoToml` | Editing and navigation | Some open cursor questions | [MISSING] |
| `viewSyntaxTree`, `viewHir`, `viewMir`, `viewItemTree`, `getFailedObligations`, `viewFileText` | Internals | Documented for debugging rust-analyzer itself | [MISSING], low priority |
| `rust-analyzer/interpretFunction` | Evaluate function without compiling | "Highly experimental" | [MISSING], avoid promoting |
| Capabilities: Hover Actions, Hover Range, Snippet `TextEdit`, CodeAction Groups, Workspace Symbols Filtering, Client Commands, Colored Diagnostic Output | | | Unverified |

Maintainer and contributor positions:
- matklad suggested editors should own persistence of the analyzer process rather than rust-analyzer running one itself, and in 2021 outlined disk spilling, saved query graph and prebuilt libraries — [RA #4712](https://github.com/rust-lang/rust-analyzer/issues/4712)
- Persistent caches were called a nice-to-have needing substantial design work in January 2024 (davidbarsky) — [RA #4712](https://github.com/rust-lang/rust-analyzer/issues/4712)
- Veykril pointed to existing workspace-check and separate target directory config options for those affected — [RA #4712](https://github.com/rust-lang/rust-analyzer/issues/4712)
- For stale inlay hints after agent edits, matklad suggests trying `files.watcher`, noting the server watcher may be racy — [matklad](https://matklad.github.io/2026/08/21/rust-glancer.html). The default is `"client"` — [RA config](https://rust-analyzer.github.io/book/configuration.html)
- Defaults worth knowing: `lru.capacity` null (docs say 128), `check.workspace` true, `procMacro.processes` 2, `cachePriming.enable` true, lens run on, reference lenses off, `extraTestBinaryArgs` `["--nocapture"]` — [RA config](https://rust-analyzer.github.io/book/configuration.html)

### Inferences
- `memoryUsage` was named in the assignment but is absent from the method list read here, and the forum says the related command does nothing. Rust Code should therefore read memory from the OS process rather than plan on this request.
- Hide `viewHir`, `viewMir` and `viewSyntaxTree` behind a developer menu, since the docs position them as server debugging tools; spend UI effort on expansion, layout and crate graph.
- Implementing flycheck cancel plus serverStatus would give visible control over latency, which most complaints concern. A client designed to reuse one server per workspace follows matklad's point that process persistence belongs to the editor.
- `extraTestBinaryArgs` and `runnables.command` overlap with the existing nextest and run configuration, so conflicts should be tested.

### Gaps
- No maintainer list of client mistakes or underused extensions was found, in docs or issues reached. A Zulip or issue-level search could change this. The method list was read through a summarizing fetch, so it may omit items; verify against the page before implementation.
- `syntaxTree` naming, exact semantic token modifiers (including `unsafe`), and the rust-analyzer target-directory option name were not verified here.
