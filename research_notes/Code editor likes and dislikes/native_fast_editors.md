# Developer Sentiment on Fast Native Code Editors (Zed, Sublime Text, Lapce, Atom/Pulsar, xi) — Lessons for a Rust/egui VS Code-style Editor

Research date: 2026-10-24. Tool budget limited (~18 calls); several items flagged as gaps. Star counts captured from GitHub pages on the research date.

## Zed: what users love, what they dislike, top requests, extension model

### Takeaway
Zed (~92k GitHub stars) is the reference Rust editor. People love its speed, "good taste" codebase, collaboration, and LLM integration. Complaints cluster on Windows/Linux maturity, ecosystem gaps vs VS Code, unconsented network downloads (Node/npm, LSP binaries), VC/AI monetization worries, TypeScript performance/RAM, and a narrow WASM extension API with no custom UI. The most-reacted open issues are mostly "VS Code parity + control" items: offline mode, consent for downloads, multi-window, test runner, settings sync, smooth scrolling.

### Cited Findings
**Scale / adoption**
- zed-industries/zed: 92.1k stars, 11.2k forks, ~2.3k open issues (Oct 2026) — [GitHub](https://github.com/zed-industries/zed)
- Sequoia invested in Zed Industries; HN thread (Aug 2025) dominated by "should an editor be VC-backed?" debate — [HN via hn.svelte.dev](https://hn.svelte.dev/item/44961172)

**Loves**
- "it all screams 'good taste'" (mccoyb, on the codebase); "It really is a delight to use" (zkoch); "fast, easy to configure", collaboration compelling (Jtsummers); runs ~10 instances without lag (decentrality); "flawless in co-operating with most LLM models" (chewz) — [HN 44961172](https://hn.svelte.dev/item/44961172)
- Native debugger shipped June 2025: ~8 months, ~1,000 commits, >25k LOC, community-led (Remco Smits); DAP client; Rust, C/C++, JS, Go, Python out of the box; "locators" turn build configs into debug configs; planned: watch lists, memory, disassembly, stack-trace views — [Phoronix](https://www.phoronix.com/news/Zed-Debugging-Merged); [Zed blog](https://zed.dev/blog/debugger); [It's FOSS](https://itsfoss.com/news/zed-debugger-support/)
- After AI backlash Zed added global `"disable_ai": true` (July 2025) plus an onboarding opt-out toggle — [Phoronix](https://www.phoronix.com/news/Zed-Option-Disable-AI-Features); [Neowin](https://www.neowin.net/news/its-now-possible-to-disable-all-ai-features-in-zed-editor/)

**Dislikes**
- Windows: GA October 2025 after a long delay. The co-founder said native Rust on Windows was hard; The Register blamed Windows/Unix differences — [Neowin](https://www.neowin.net/news/windows-slows-zeds-development-as-platform-differences-bite/); [The Register](https://www.theregister.com/2025/09/30/zed_windows_beta_sneaks_out/)
- Windows reviews in 2026: a Jan 2026 blog calls it a workable VS Code alternative, but bug-prone. A July 2026 review calls it a "real contender" with an official installer. A 2026 review site says it is "not yet ready" for Windows users or for teams that depend on VS Code extensions — [dotnetperls](https://dotnetperls.com/2026_1_28_zed-editor-windows); [freshbrewed](https://freshbrewed.science/2026/07/09/zed.html); [aisotools](https://aisotools.com/blog/zed-review-2026)
- Windows launch asked for feedback on WSL, IME/keyboard layouts, multi-monitor, and high-refresh displays. These are the known hard areas for native GPU editors — [hunted.space](https://hunted.space/product/zed/launches/zed-for-windows)
- Font rendering: Zed uses grayscale glyph AA. DirectWrite subpixel (ClearType-style) AA needs RGB coverage plus the foreground color, which Zed's text system does not supply. A commit says matching VS Code quality would need a larger refactor. Result: fuzzier text on low-DPI Windows/Linux screens — [Zed commit patch](https://git.secluded.site/zed/commit/c0259a448d6052222e8492538594e3d0e88733ae.patch)
- Performance outliers: TypeScript work can "balloon to 15gb of ram" (x0x0). One Linux user found it "so slow as to be unusable" (duped); a macOS user disagreed — [HN 44961172](https://hn.svelte.dev/item/44961172)
- Consent/network: users object to "downloading and running third party software without asking the user" (bigstrat2003, arkits, reitanuki). Issue #12589 "Zed downloads NodeJS binary and npm packages from Internet without user's consent" is in the top-reacted open list — [HN 44961172](https://hn.svelte.dev/item/44961172); [GitHub issues](https://github.com/zed-industries/zed/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc)
- AI/monetization: users resist paying for AI-centric tiers (reitanuki, runako). One wants monetization "beyond making a spread on Anthropic API pricing" (koito17). An enterprise user can't buy it because it counts as an unapproved AI service (trenchpilgrim). The agent "kept asking me for confirmation" (resonious) — [HN 44961172](https://hn.svelte.dev/item/44961172)
- Telemetry: client metrics and crash reports can be disabled. Server-side data is collected when using hosted AI/collab, and is "Required for these features to function." Users asked whether background AI services remain after disabling (discussion #16750, no clear answer) — [Zed telemetry docs](https://zed.dev/docs/telemetry.html); [GitHub discussion #16750](https://github.com/zed-industries/zed/discussions/16750)

**Most-reacted open issues (GitHub sort by +1 reactions, Oct 2026; numbers shown are likely comment counts, not reactions)**
- #4355 Smooth scrolling; #5242 Test runner integration; #21538 jj (Jujutsu) SCM; #4930 Vim easymotion; #21208 Webview via extensions; #9662 secondary windows / multi-monitor; #14801 flash.nvim-style search; #9459 open `.code-workspace` files; #7808 "Make the Python experience wonderful"; #9721 Difftastic diffs; #12589 Node/npm downloads without consent; #6722 drag tab to new window; #11889 iPad; #4867 per-file tab settings; #10647 configurable mouse bindings; #5117 traditional Find All References view; #5010 built-in Settings Sync; #18698 TypeScript server slow vs VS Code; #24878 Offline support meta-issue; #34813 Git 3-way merge; #13300 per-project themes; #14473 kinetic scrolling on Wayland; #7121 disable unwanted panels completely; #6754 `--offline` mode; #7409 localization — [GitHub issues sorted by reactions](https://github.com/zed-industries/zed/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc)

**Extension model (WASM)**
- Extensions are written in Rust and compiled to Wasm with the WebAssembly Component Model (WIT), then run in Wasmtime. They are sandboxed, so a crash should not take down the editor, and they hot-reload. API crate: `zed_extension_api` — [Zed blog: language extensions pt 1](https://zed.dev/blog/language-extensions-part-1); [plushcap summary of "Life of a Zed extension"](https://www.plushcap.com/content/zed/blog/life-of-a-zed-extension-rust-wit-wasm)
- Main extension points: languages (tree-sitter grammar + LSP adapter), themes, snippets, slash commands, debug adapters. Extensions cannot modify the UI or perform arbitrary operations (2024 summary). Zed says it plans to widen the surface to "customize Zed's UI" — [plushcap 2024-10 summaries](https://www.plushcap.com/companies/zed/blog/summaries/2024/10); [zed.dev/extensions/html](https://zed.dev/extensions/html)
- No WebViews: one third-party analysis says this rules out Markdown preview, Swagger UI, and other embedded-browser tools via extensions (unverified). Issue #21208 "Webview via Extensions" is a top request — [devradar](https://devradar.dev/tools/zed-ai); [GitHub issues](https://github.com/zed-industries/zed/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc)
- Ecosystem pull of VS Code: "being able to use JS to extend your editor is quite valuable" (ares623); VS Code "came along with a thriving extension ecosystem" (Already__Taken) — [HN 44961172](https://hn.svelte.dev/item/44961172)

### Inferences
- A new Rust editor can win on speed, but users judge it on parity basics: debugger, test runner, Find References panel, multi-window, settings sync, and workspace-file import. Zed took ~3 years to ship a debugger.
- "Control" is a feature. Offline mode, explicit consent before downloading toolchains or LSPs, a full AI kill switch, and opt-in telemetry are among the most-wanted items. Bake them in from day one.
- Text rendering quality on low-DPI Windows matters. egui's default font rasterization is also grayscale (ab_glyph-based), so expect the same complaint unless subpixel/LCD AA or DirectWrite/FreeType hinting is added.
- A WASM-sandboxed extension API that cannot contribute UI (panels, views, webviews) caps ecosystem growth. Plan early for declarative UI contributions or a webview escape hatch.

### Gaps
- Exact +1 reaction counts for Zed issues were not visible in the fetched page.
- No primary data on Zed's user counts or Windows market share.
- Multiplayer/collab sentiment beyond one HN comment, and Zed account-requirement complaints, were not separately sourced.
- Whether Zed's extension API gained UI/panel contribution points in 2025–2026 is unconfirmed.

## Sublime Text: why loyal users stay and why people left

### Takeaway
Sublime remains the speed and stability benchmark ("blissfully fast always", crash recovery, large files, many instances). Users leave over a perceived stagnant core (one feature release since ST4, per a critic), volunteer-maintained LSP, abandoned plugins, Package Control breakage on new macOS, and paid upgrade licensing with nagging upgrade notices.

### Cited Findings
- Stays: "faster than Zed, works on Linux/Win/MacOS, and is decently customizable" (toastercat); "blissfully fast always", crash recovery praised (mstade); used "for opening large files" (dewey); 19 instances with no lag (bogwog) — [HN 44961172](https://hn.svelte.dev/item/44961172)
- Leaves: "exactly one release with new features" since ST4, "all but dead" (8fingerlouie); plugins "not being updated for years" (AstroBen); LSP is "behind a volunteer plugin instead of integrated directly", so the user moved to Zed (WD-42); "dark patterns" in upgrade notices (dpkirchner), which Sublime dev ben-schaaf acknowledged while defending the license model — [HN 44961172](https://hn.svelte.dev/item/44961172)
- Forum thread "Should I invest into Sublime anymore?" (Dec 2024): it "never became a decent IDE". Package Control failed on macOS for years, attributed to OpenSSL ABI changes breaking PC3, with PC4 unsigned and undistributed. Users are reluctant to pay for the next license. Stayers cite speed, "no-nonsense" UI, easier customization than VS Code, a small responsive team, and privacy vs Microsoft telemetry/AI training. No official dev replies in thread — [Sublime Forum](https://forum.sublimetext.com/t/should-i-invest-into-sublime-anymore/74868)
- Older forum: a user with a 2019 license was told the next upgrade needs a new license, and moved to fast-moving VS Code — [Sublime Forum (older)](https://forum.sublimetext.com/t/im-very-sad-to-say-goodbye-to-sublime-text/38888)
- Counterflow: a 2025 author left VS Code for ST4, citing a $99 license and cold start "under a second" on a MacBook Pro — [pivotlog](https://pivotlog.net/switched-away-from-vs-code/)
- Counterpoint: VS Code, JetBrains, and Neovim "have caught up"; the author prefers waiting two seconds for an editor that does more (Sep 2025) — [Medium](https://medium.com/@brevis08/why-i-stopped-using-sublime-text-221584ef041f)
- Maintenance: last stable build May 2025, dev builds through July 2025 (secondary summary) — [daily.dev](https://daily.dev/posts/they-don-t-make-em-like-sublime-text-anyone-odl3ghlom)

### Inferences
- Sublime's durable assets map to features a new editor should copy: sub-second cold start, instant Goto Anything/command palette, multi-cursor, hot-exit crash recovery, and graceful huge-file handling.
- Ship first-class built-in LSP and DAP. Delegating core IDE features to volunteer plugins is a churn driver.
- A plugin ecosystem needs a maintained registry and a stable ABI/transport. Package Control's OpenSSL breakage shows how fragile package-manager infrastructure is.

### Gaps
- No official Sublime HQ statement on roadmap or release cadence was found. Exact current license price and terms were not confirmed from sublimetext.com.
- Goto Anything and multiple-cursor praise is widely known, but not separately sourced here.

## Lapce: architecture, adoption, stall?

### Takeaway
Lapce (39k stars) proves interest in a pure-Rust, GPU (wgpu), Floem-based editor with xi ropes and WASI plugins. Momentum has faded: big releases v0.3.0 (Oct 2023, Floem rewrite) and v0.4.0 (Apr 2024) drew the most engagement. Since v0.4.2 (Aug 2024), releases are small, packaging-focused, and months apart (v0.4.3 Jun 2025, v0.4.4/0.4.5 Aug/Sep 2025, v0.4.6 Jan 2026), and it is still pre-1.0.

### Cited Findings
- Stats: 39.0k stars, 1.4k forks, 831 open issues, 71 PRs, 3,653 commits. Pure Rust, Floem UI, xi-editor ropes, wgpu rendering, WASI plugins (C, Rust, AssemblyScript), built-in LSP, modal editing, remote dev, terminal — [GitHub lapce/lapce](https://github.com/lapce/lapce)
- Release history: v0.3.0 31 Oct 2023 (Floem rewrite, DAP; 130 reactions); v0.4.0 24 Apr 2024 (Floem port, experimental Copilot, line wrap, Linux packages; 83 reactions); v0.4.1 Aug 2024 (tree-sitter grammar download, call hierarchy); v0.4.2 Aug 2024 (66 reactions); v0.4.3 Jun 2025 (Find References / Go To Implementation panels); v0.4.4 30 Aug 2025 (7 reactions); v0.4.5 05 Sep 2025 (20 reactions); v0.4.6 21 Jan 2026 (bug fixes); nightly built 09 Oct (2026 inferred) — [GitHub releases](https://github.com/lapce/lapce/releases)
- User sentiment, Jan 2026 Reddit: Lapce "looks nice on the paper, but it's really rough on the edge each time I try it" — [Reddit snapshot](https://reddit.sentinel-team.org/posts/1qhuhw3/snapshots/2026-01-21T14%3A00%3A17.20303Z)
- Still packaged by distros (ALT Linux 0.4.5, Dec 2025) — [ALT Linux](https://packages.altlinux.org/en/sisyphus/srpms/lapce/)

### Inferences
- Lapce shows the classic trap: UI toolkit rewrites (Druid to Floem) consumed the time that should have gone to editor features. This mirrors xi/Druid. For an egui project, use the toolkit as-is and avoid forking or rewriting it mid-stream.
- Without a funded team, a Rust editor plateaus at "nice but rough". Prioritize polish of the core loop (typing, scrolling, search, LSP) over breadth.
- WASI plugins did not produce a large ecosystem. A plugin model alone does not attract authors; reach, docs, and UI contribution points matter.

### Gaps
- No official statement on maintainer bandwidth, funding, or roadmap was found. Plugin registry size is unknown.

## Atom/Pulsar: why Atom died, lessons

### Takeaway
GitHub's official reason (June 2022, sunset Dec 2022) was stagnation plus a strategic focus on VS Code/Codespaces after the Microsoft acquisition. Community commentary blames poor performance (Electron, DOM, GC) and the gravitational pull of VS Code's extension ecosystem. Zed was founded by ex-Atom developers explicitly to escape those performance limits. Pulsar continues as a community Electron fork with a smaller footprint.

### Cited Findings
- GitHub: Atom "has not had significant feature development for the past several years", only maintenance and security updates. The focus moved to Codespaces/VS Code — [GitHub blog](https://github.blog/2022-06-08-sunsetting-atom/embed/); [Visual Studio Magazine](https://visualstudiomagazine.com/Articles/2022/06/09/atom-sunsetting.aspx)
- Secondary: performance "sub par compared to rival code editors" was the primary reason users moved. VS Code, also Electron, handled large files much better — [It's FOSS](https://news.itsfoss.com/atom-being-discontinued); [mjtsai](https://mjtsai.com/blog/2022/06/10/sunsetting-the-atom-text-editor)
- Atom's legacy is Electron itself — [ghacks](https://www.ghacks.net/?p=179057)
- Zed/GPUI origin: the Atom team found Electron GC and DOM relayouts made consistent frame rates hard. At 120Hz an app "only has 8.33ms per frame" — [Zed blog "Leveraging Rust and the GPU to render UIs at 120 FPS" (Mar 2023)](https://zed.dev/blog/videogame)
- Pulsar: community fork after the Dec 2022 EOL; aims for Atom parity and architecture modernization; Electron-based; plugin update cycles hurt by Atom's death (older coverage, 2022–2024) — [It's FOSS](https://itsfoss.com/news/pulsar-editor/); [Slashdot Jan 2024](https://tech.slashdot.org/story/24/01/01/016215/since-the-demise-of-atom-pulsar-offers-an-alternative-code-editor)

### Inferences
- Lessons: (1) performance debt in the core architecture is unrecoverable later. (2) Corporate-owned editors die when strategy shifts, so governance and licensing are trust signals. (3) A hackable editor without a fast core loses to a slightly less hackable but fast one. (4) Ecosystem lock-in (VS Code extensions) is the main moat. Consider compatibility layers, such as VS Code themes/snippets/TextMate grammars, LSP, DAP, and `.code-workspace` import.

### Gaps
- No 2025–2026 Pulsar release, contributor, or funding data was found.
- Panic Nova (macOS-native, paid) and CotEditor (macOS-native, free) were not covered by search results. No sourced sentiment is available for either.

## Technical lessons: xi retrospective and Zed engineering

### Takeaway
Use a rope/B-tree with cheap immutable snapshots (Zed SumTree; xi rope). Avoid xi's mistakes: multi-process frontend/core split, pervasive async, JSON-RPC everywhere, CRDT-for-everything, and a plugin API overfit to syntax highlighting. Use tree-sitter rather than regex grammars, and render on the GPU with a glyph atlas.

### Cited Findings
**xi retrospective (Raph Levien, June 2020 — older but canonical)**
- The rope is a success: "there aren't any cases where it performs _badly._", and in Rust its complexity is contained behind an interface — [xi retrospective](https://raphlinus.github.io/xi/2020/06/27/xi-retrospective.html)
- CRDT: expensive and a poor fit; indentation "basically doesn't fit nicely in the CRDT model at all" — [xi retrospective](https://raphlinus.github.io/xi/2020/06/27/xi-retrospective.html)
- Async/multiprocess: async "made everything more complicated". Splitting frontend and core into processes was a mistake. Plugin processes (LSP-like) were OK — [xi retrospective](https://raphlinus.github.io/xi/2020/06/27/xi-retrospective.html)
- JSON-RPC: slow in Swift, serde bloated binaries — [xi retrospective](https://raphlinus.github.io/xi/2020/06/27/xi-retrospective.html)
- Plugin API became overspecialized for syntax highlighting. Design plugins from broad use cases — [xi retrospective](https://raphlinus.github.io/xi/2020/06/27/xi-retrospective.html)
- Regex (TextMate/Sublime) highlighting is slow and imprecise. Prefer tree-sitter/Lezer. Incremental async highlighting kept typing responsive — [xi retrospective](https://raphlinus.github.io/xi/2020/06/27/xi-retrospective.html)
- Native toolkits perform poorly for text. GPU is "essentially required for good GUI performance", which led to Druid — [xi retrospective](https://raphlinus.github.io/xi/2020/06/27/xi-retrospective.html)

**Zed Rope & SumTree (Apr 2024)**
- Compared alternatives: gap buffer (Emacs), piece table (VS Code variant), custom trees (Vim), ropes (Helix, Zed). Ropes are "good at everything, excellent at nothing" (ropey maintainer), with consistent O(log N) edits — [Zed Decoded: Rope & SumTree](https://zed.dev/blog/zed-decoded-rope-sumtree)
- Concurrent access was a hard requirement. Snapshots cost an `Arc` refcount bump, and each edit sends a snapshot to a background thread for tree-sitter reparse — [same](https://zed.dev/blog/zed-decoded-rope-sumtree)
- SumTree is a B+ tree with generic summaries (TextSummary: bytes, UTF-16 length, lines, longest row). Cursors seek one dimension while aggregating others, which handles offset/point/UTF-16 conversions for LSP. It has 20+ uses: project file list, git blame, diagnostics, DisplayMap (folds, soft wrap, inlay hints, tabs, blocks). It is "the soul of Zed" (Nathan Sobo) — [same](https://zed.dev/blog/zed-decoded-rope-sumtree)

**GPUI rendering (Mar 2023)**
- Game-style renderer with a dedicated shader per primitive (rects, shadows, text, icons, images). Rounded rects use SDFs; shadows use an erf closed form. Text shaping uses OS APIs with per-(text,font) caches. Glyphs are rasterized by the OS to alpha-only, with up to 16 subpixel-position variants, packed into an atlas (etagere) and drawn in one instanced call. Layout is Flutter-style constraints. The Scene draws in fixed primitive order, with layers for overlays — [Zed blog](https://zed.dev/blog/videogame)

### Inferences (for a Rust + egui editor)
- Buffer: adopt `ropey` or a SumTree-like B-tree with multi-dimensional summaries (byte, char, UTF-16, line). Make snapshots `Arc`-cheap so tree-sitter, LSP sync, search, and git diff run on background threads without locking the UI.
- Keep a single process. Use threads and channels instead of async-everywhere, and isolate only plugins and language servers out-of-process or in a WASM sandbox.
- Don't build a CRDT unless collaboration is a core goal. Use OT/CRDT only for a collab layer, not the core buffer.
- egui is immediate-mode. Virtualize the editor view (render only visible lines), cache shaped lines and galleys keyed by content hash, and avoid relayout of the whole document per frame. GPUI's per-line shape cache is the model.
- Text quality: egui's grayscale atlas will draw the same low-DPI blur complaints as Zed on Windows. Consider subpixel positioning variants and LCD/ClearType AA as a differentiator.
- Use tree-sitter for highlighting/structure and LSP/DAP for intelligence. Support TextMate grammars only as a compatibility fallback.

### Gaps
- Zed engineering posts on Linux/Windows text rendering (DirectWrite, cosmic-text/FreeType) were not fetched beyond the commit patch.
- No benchmarks were found comparing egui-based editors to Zed or Sublime.
