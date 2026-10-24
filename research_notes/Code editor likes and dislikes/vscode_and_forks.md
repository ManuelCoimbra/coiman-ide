# VS Code, its AI forks (Cursor, Windsurf, Trae), VSCodium/Open VSX, and the extension ecosystem: developer sentiment

Research date: 2026-10-24. Install counts and GitHub reaction counts were pulled live on that date from the public Visual Studio Marketplace gallery API (`marketplace.visualstudio.com/_apis/public/gallery/extensionquery`, sorted by install count) and the GitHub search API (`repo:microsoft/vscode`, sorted by 👍 reactions). Treat them as point-in-time snapshots.

## 1. What developers praise about VS Code (market position, strengths)

### Takeaway
VS Code is still the default editor by a wide margin. Stack Overflow 2025 puts it at 75.9% usage, about 2.6x the next tool. People praise its ecosystem (>100k extensions), free cross-platform availability, LSP-based language support, and Remote/WSL/Dev Containers. Many extensions with tens of millions of installs are Microsoft's own language and remote tooling, so a lot of that praise is really praise for Microsoft-maintained extensions.

### Cited Findings
- SO Developer Survey 2025 IDE usage (all respondents / professionals): VS Code 75.9% / 76.2%; Visual Studio 29% / 29.7%; Notepad++ 27.4%; IntelliJ IDEA 27.1%; Vim 24.3%; **Cursor 17.9% / 19.3%**; Android Studio 15%; PyCharm 15%; Neovim 14%; Sublime Text 10.5%; Claude Code 9.7%; **Zed 7.3%**; **VSCodium 6.2%**; **Windsurf 4.9%**; RustRover 3.2%; Cline/Roo 2.2%; Aider 1.9%; **Trae 0.8%**. Based on ~26k respondents. — [Stack Overflow Survey 2025 – Technology](https://survey.stackoverflow.co/2025/technology/); [Visual Studio Magazine](https://visualstudiomagazine.com/articles/2025/08/01/stack-overflow-dev-survey-visual-studio-vs-code-hold-of-ai-ides-to-remain-on-top.aspx)
- VS Code usage rose from 73.6% (2024) to 75.9% (2025). It has been the most-used IDE for 5 straight years and is also ranked "most desired"; IntelliJ IDEA is second in Admired/Desired. — [Visual Studio Magazine](https://visualstudiomagazine.com/articles/2025/08/01/stack-overflow-dev-survey-visual-studio-vs-code-hold-of-ai-ides-to-remain-on-top.aspx); [secondtalent](https://www.secondtalent.com/resources/most-used-ides/)
- The marketplace passed 100,000 extensions (Microsoft milestone, May 2025). Comparisons with Zed still name the ecosystem as the main reason to stay on VS Code. — [tms-outsource](https://tms-outsource.com/blog/posts/vs-code-extensions/); [dev.to Zed vs VS Code 2026](https://dev.to/truongandev/zed-vs-vs-code-in-2026-is-it-worth-switching-17gm)
- Remote development is a heavily used pillar: WSL 41.1M installs (rating 4.79), Dev Containers 42.4M, Remote-SSH 38.2M, Remote Explorer 30.6M (marketplace API snapshot, see section 3).
- Features that were once top requests and later shipped are now core strengths. Settings Sync (#2743, 746 👍, shipped 2020). Multi-root workspaces (#396, 2618 👍, 2017). Git status in Explorer (#178, 2285 👍). Terminal tabs (#10546, 2411 👍, 2021). 3-way merge editor (#37350, 1464 👍, 2022). Floating windows (#10121, 2876 👍, closed Dec 2023). — [GitHub microsoft/vscode closed issues sorted by reactions](https://github.com/microsoft/vscode/issues?q=is%3Aissue+is%3Aclosed+sort%3Areactions-%2B1-desc)
- Openness move: Microsoft open-sourced the GitHub Copilot Chat extension under MIT on 30 Jun 2025, including agent mode, the context sent to LLMs, and system prompts. Inline completions were to move into it, and the backend stays closed. Microsoft's own wording is that VS Code is "built on open source" rather than "is open source". — [Visual Studio Magazine](https://visualstudiomagazine.com/articles/2025/06/30/vs-code-goes-transparent-as-open-source-ai-editor.aspx); [BleepingComputer](https://bleepingcomputer.com/news/security/microsoft-open-sources-vs-code-copilot-chat-extension-on-github)

### Inferences
- The "free + extensible + good LSP + remote" bundle is the baseline. A new editor that lacks LSP, a debugger protocol (DAP), remote/WSL, and a plugin system will not pull VS Code users, however fast it is.
- VS Code is 2–3 years old in most surveys' "admired" sense, but no competitor has broken 20% usage. Cursor is the closest at 18%, and it is itself a VS Code fork.

### Gaps
- Exact SO 2025 Admired/Desired percentages for VS Code, Zed, and Neovim were in chart images and could not be extracted.
- The SO 2026 survey (likely released mid-2026) was not checked.

## 2. Most common complaints about VS Code

### Takeaway
The recurring complaints fall into six groups:
1. Electron resource use and sluggishness, which drives switches to Zed.
2. An AI/Copilot push that users cannot fully turn off.
3. Telemetry that is on by default.
4. Microsoft lock-in through proprietary extensions and marketplace terms.
5. Extension-supply-chain security.
6. Settings and layout complexity: there is no workspace-level UI font, no settings inheritance, and no per-platform settings.

### Cited Findings
**Performance / Electron**
- First-person switch stories cite VS Code memory use, "countless background processes", battery drain, and slowness "even on my fast Mac". Fans spin down after switching to Zed, whose GPU renderer is GPUI. Zed 1.0 shipped late April 2026. Benchmark claims (3–4x to 16x less RAM, 2ms vs 12ms input latency) come from SEO-style posts and are unverified. — [dev.to yash37158](https://dev.to/yash37158/why-i-shifted-from-vs-code-to-zed-on-mac-a-developers-story-38jp); [Alex Hyett](https://www.alexhyett.com/newsletter/switching-to-zed-for-coding/); [falcao.org Zed 1.0](https://falcao.org/posts/zed-1-0/); [tech-insider](https://tech-insider.org/zed-vs-vscode-2026/)
- Long-open issue #28795 "Editor: scroll jumps randomly (related to Chrome, Electron, xinput)" has 456 👍 (opened 2017). — [GitHub #28795](https://github.com/microsoft/vscode/issues/28795)

**AI / Copilot push**
- VS Code began adding "Co-authored-by: Copilot" to commits by default on 16 Apr 2026, even when AI was not used and even with the disable-AI setting on. The VS Code lead apologized on HN ("a mistake made without sufficient upfront validation"). It was reverted to off by default in 1.119 around 3 May 2026. — [Windows Central](https://windowscentral.com/software-apps/this-could-cost-people-their-jobs-vs-code-added-copilot-as-co-author-without-permission-or-notice); [WebProNews](https://www.webpronews.com/vs-codes-sneaky-copilot-co-authorship-how-microsoft-rewrote-git-histories-and-lost-developer-trust/)
- An HN post "switched to Zed" from 6 Jan 2026 drew 250+ comments. Complaints: AI UI persists even when disabled, and each update adds AI features on by default, so settings files need constant upkeep. (Byteiota is a weaker outlet.) — [byteiota](https://byteiota.com/vscode-ai-bloat-drives-developers-to-zed-why-they-switch/)
- A Copilot icon reappeared in VS Code after the Copilot extension was uninstalled (Andi McClure, Jan 2025). The most-upvoted GitHub community discussion asked to stop Copilot-generated issues and PRs. — [The Register, Sep 2025](https://www.theregister.com/2025/09/05/github_copilot_complaints/)
- The repo's live issue stream (Oct 2026) is dominated by Agents window, Copilot CLI agent host, Voice Mode, MCP gallery, plugins, and worktree sessions. This shows how far core-team focus has moved to AI. — [GitHub microsoft/vscode newest issues](https://github.com/microsoft/vscode/issues)
- Copilot extensions have low marketplace ratings despite high installs: Copilot Chat 79.0M installs, rated 2.78; Copilot 74.8M, 4.08; OpenAI Codex 15.9M, 2.86; Gemini Code Assist 5.6M, 1.88 (marketplace API snapshot).

**Telemetry**
- Issue #176269 "Require user consent before sending any telemetry" is open with 1,005 👍 (2023). — [GitHub #176269](https://github.com/microsoft/vscode/issues/176269)

**Lock-in / proprietary extensions**
- Microsoft's C/C++ extension (cpptools, 106M installs) added a runtime check in v1.24.5 (3 Apr 2025) that blocks forks such as Cursor and VSCodium. The license had restricted this since 2020–2021, but it was not enforced until then. Developers asked the FTC to look into it. Cursor said it would move to open-source alternatives. Pinning v1.24.3 was a workaround. — [The Register](https://www.theregister.com/2025/04/24/microsoft_vs_code_subtracts_cc_extension/); [Cursor forum](https://forum.cursor.com/t/c-c-extension-usage-restriction-message-appears-in-cursor/75902.md); [Slashdot](https://developers.slashdot.org/story/25/04/25/2139216/devs-sound-alarm-after-microsoft-subtracts-cc-extension-from-vs-code-forks)
- Marketplace terms allow use only with Microsoft-branded VS Code. Remote Development, Live Share, and some GitHub integrations are not on Open VSX. — [ArchWiki](https://wiki.archlinux.org/title/Code); [linuxcommandlibrary codium](https://linuxcommandlibrary.com/man/codium)
- Proprietary Microsoft language extensions are among the lowest-rated popular extensions: Pylance 209M installs, rated 2.95; C# 44.0M, 2.73; C# Dev Kit 18.5M, 2.32; Jupyter 110.7M, 2.64; Python Environments 62.3M, 1.3 (marketplace API snapshot).

**Settings / customization complexity** (top open issues by 👍; see section 4)
- Requests include: a separate workbench font size (#519, 3,790 👍, the #1 open issue since 2015); extending settings files (#15909, 1,368); per-platform settings (#5595, 1,150); local workspace settings (#40233, 1,043); enabling/disabling extensions from a config file (#40239, 840); variables in settings (#2809, 742); and an app-data folder layout (#3884, 708). — [GitHub open issues sorted by reactions](https://github.com/microsoft/vscode/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc)

**Extension security** — see section 6.

### Inferences
- Several pain points line up directly with a Rust/egui editor's natural strengths: low RAM, fast startup, no forced AI, and no telemetry by default (opt-in, per #176269). "AI optional and truly removable" is a real differentiator, as the co-author incident shows.
- The settings model is a long-standing weak spot. Layered settings (global / platform / profile / workspace / local-uncommitted), inheritance, variables, and a declarative extension enable list are cheap to design in from day 1.

### Gaps
- No rigorous independent benchmark of VS Code vs Zed RAM/latency was found; numbers are anecdotal.
- No survey quantifies the share of VS Code users bothered by telemetry or AI.

## 3. Top VS Code Marketplace extensions by installs, and the gap each fills

### Takeaway
The top ~60 extensions by installs are dominated by Microsoft language stacks: Python, Jupyter, C/C++, Java pack, C#, and Remote. Next come AI assistants, then gap-fillers the core lacks. The gap-fillers are formatters/linters (Prettier, ESLint), git history/graph (GitLens, Git Graph, Git History), a dev web server (Live Server), file icons, themes, inline diagnostics (Error Lens), indent visualization, path completion, TODO aggregation, a Vim mode, REST clients, CSV highlighting, and spell checking.

### Cited Findings (live marketplace API snapshot, 2026-10-24; installs / average rating)
Top by installs (rank order):
1. Python (ms-python) — 242.5M / 4.21 — language pack, interpreter selection, testing
2. Pylance — 209.2M / 2.95 — proprietary Python LSP (fork-blocked)
3. Python Debugger (debugpy) — 147.9M / 4.71
4. Jupyter — 110.7M / 2.64 (+ Keymap 90.3M, Renderers 89.4M, Cell Tags 70.7M, Slide Show 70.7M — auto-installed companions)
5. C/C++ (cpptools) — 106.1M / 3.42 (+ Themes 60.9M, Extension Pack 59.0M)
6. Live Server (ritwickdey) — 83.8M / 4.4 — local dev server with auto-reload for HTML/CSS/JS (core has none; Microsoft's own Live Preview has 13.9M)
7. GitHub Copilot Chat — 79.0M / 2.78; GitHub Copilot — 74.8M / 4.08
8. Prettier — 72.5M / 3.54 — opinionated formatter for JS/TS/CSS/etc.
9. CMake Tools — 64.4M / 4.47; CMake (twxs) — 46.8M
10. IntelliCode — 62.6M / 3.9 (+ API Usage Examples 47.1M)
11. Python Environments — 62.3M / 1.3
12. Language Support for Java (Red Hat) — 58.1M; Java Debugger 53.4M; Maven 49.5M; Java Test Runner 48.0M; Project Manager for Java 47.5M; Java Extension Pack 47.2M; Gradle 29.2M
13. Chinese (Simplified) Language Pack — 56.6M / 4.83 — UI localization matters at scale
14. GitLens — 53.5M / 3.38 — blame annotations, history, and graph (core git UI is thin)
15. ESLint — 53.5M / 4.29
16. Docker — 53.1M / 4.48 (+ Container Tools 19.1M)
17. C# — 44.0M / 2.73; C# Dev Kit 18.5M / 2.32; .NET Install Tool 30.9M
18. Code Runner — 42.5M / 4.4 — one-click "run this file" for many languages (core has no simple run)
19. Dev Containers 42.4M / 4.05; WSL 41.1M / 4.79; Remote-SSH 38.2M / 3.59; Remote-SSH config editing 36.0M; Remote Explorer 30.6M
20. GitHub Pull Requests — 37.2M / 3.93
21. Material Icon Theme — 36.0M / 4.94 — file icons (default icon set considered weak); vscode-icons 24.7M / 4.77
22. HTML CSS Support — 30.0M; YAML (Red Hat) — 29.7M
23. **Claude Code for VS Code (Anthropic) — 28.6M / 3.72**
24. Auto Rename Tag — 26.2M — paired HTML/XML tag editing
25. isort — 25.9M; Rainbow CSV — 24.7M / 4.79; Live Share — 24.3M / 4.24; WakaTime — 23.1M; PowerShell — 22.4M; JS (ES6) snippets — 22.2M; GitHub Theme — 20.4M; Go — 20.0M; Path Intellisense — 19.6M / 4.85; Python Indent — 19.0M; Code Spell Checker — 18.5M / 4.47; Django — 18.2M

Other notable extensions (same snapshot):
- Git History 17.4M / 4.44; **Git Graph 15.6M / 4.94** — commit graph visualization (core lacks it; see open issue #179053, 611 👍)
- OpenAI Codex 15.9M / 2.86; Cline 5.7M / 4.01; Continue 4.4M / 3.19; Gemini Code Assist 5.6M / 1.88
- EditorConfig 15.3M; Markdown All in One 14.8M / 4.69; Tailwind CSS IntelliSense 14.8M; Live Preview (MS) 13.9M
- indent-rainbow 13.4M / 4.82 — colored indentation levels (Python/YAML readability)
- One Dark Pro 12.9M / 4.67; Dracula 11.1M / 4.78 — themes
- CodeLLDB 11.1M / 4.93 — native debugger for Rust/C++ (key for Rust users)
- **Error Lens 10.2M / 4.87** — shows diagnostics inline at end of line (very highly rated; often cited as "should be built-in")
- Vim 9.2M / 4.09 — modal editing (core open issue #114851 "VIM mode like sublime", 1,370 👍)
- Todo Tree 7.9M / 4.71 — aggregates TODO/FIXME across the workspace
- REST Client 7.7M / 4.91; Thunder Client 7.7M / 2.37 (rating fell after a move to paid tiers)
- Project Manager 7.6M; Bookmarks 5.2M
- **rust-analyzer 6.8M / 4.8**; Even Better TOML 5.0M
- Draw.io Integration 4.2M / 4.91

Marketplace-wide (Koi, 2025): ~60,000 extensions, ~45,000 publishers, only ~1,800 verified, 3.3B total installs. — [Koi Security](https://www.koi.security/blog/2-6-exposing-malicious-extensions-shocking-statistics-from-the-vs-code-marketplace)
Language-extension snapshot (30 Apr 2025, older): Python 167.0M, Go 15.7M, Dart 11.9M. Compare with today's 242.5M and 20.0M to see growth. — [glukhov.org](https://glukhov.org/post/2025/05/programming-languages-frameworks-popularity)

### Inferences
- Built-in features that would absorb the top gap-fillers:
  - Inline diagnostics (Error Lens).
  - A git blame/graph/history UI (GitLens, Git Graph, Git History; core issue #179053).
  - Path completion.
  - Workspace TODO view.
  - Indent-level coloring and rainbow brackets (VS Code later absorbed bracket-pair colorization).
  - A good default icon set.
  - Format-on-save with external formatters (Prettier).
  - A "run current file" command (Code Runner).
  - An HTTP request client (REST Client).
  - CSV column coloring.
  - Spell check.
  - A first-class modal (Vim) mode.
- High installs with low ratings mark dissatisfaction a new editor could target: Pylance 2.95, Jupyter 2.64, C# Dev Kit 2.32, Copilot Chat 2.78, Python Environments 1.3. Because these are proprietary, forks cannot use them at all. An open-LSP-first approach (basedpyright/pyright, clangd, OmniSharp/Roslyn LS) is the only workable path.
- For a Rust-written editor targeting Rust devs: rust-analyzer (6.8M, 4.8), CodeLLDB (11.1M, 4.93), and Even Better TOML are must-haves via LSP/DAP.
- Localization (Chinese language pack 56.6M) is a large but often overlooked need.

### Gaps
- Marketplace "install" counts include auto-installed dependencies (the Jupyter companions, C++ themes) and CI/automated installs. They overstate deliberate choice.
- Dedicated sentiment for Live Server, Path Intellisense, and similar extensions (why people choose them over built-ins) is inferred from function, not from user surveys.

## 4. Most-upvoted open feature requests on microsoft/vscode, and what got implemented

### Takeaway
The oldest and most-upvoted open requests are about **UI customization and configuration**: workbench font, mouse bindings, settings inheritance and per-platform settings, macros, grouped and vertical tabs, a detachable terminal, tree-sitter highlighting, project-wide diagnostics, an iPad version, and a Vim mode. Many big layout requests have shipped: floating windows, grid layout, terminal tabs, multi-root, pinned tabs, file nesting, and a private marketplace (Nov 2025).

### Cited Findings (live GitHub API, 2026-10-24)
Open, sorted by 👍 (issue | 👍 | opened):
- #519 Change font size/font of the workbench | 3,790 | 2015
- #3130 Customization of mouse shortcuts | 1,932 | 2016
- #13953 Show all errors/warnings in project for all JS/TS files, not just opened | 1,509 | 2016
- #114851 VIM mode like Sublime (built-in) | 1,370 | 2021
- #15909 Extend from other settings files | 1,368 | 2016
- #70764 VS Code for iPad | 1,347 | 2019
- #4490 Macro recording | 1,228 | 2016 (also #871 macro-like keybindings, 699)
- #5595 Scope settings by platform | 1,150 | 2016
- #54272 Markdown in diagnostics messages | 1,052 | 2018
- #40233 Local (uncommitted) workspace settings | 1,043 | 2017
- #176269 Require consent before sending any telemetry | 1,005 | 2023
- #43188 Multiple workspaces in the same window | 944 | 2018
- #100335 Chrome-like grouped tabs | 854 | 2020
- #40239 Enable/disable extensions from config file | 840 | 2017
- #34442 Extract (detach) the integrated terminal | 810 | 2017
- #2809 Variables in settings values | 742 | 2016
- #50140 Syntax highlighting with tree-sitter | 739 | 2018
- #35744 Customizable status bar item locations | 719 | 2017
- #3884 Revisit folder structure for app data/settings/extensions | 708 | 2016
- #108264 Vertical tabs | 659 | 2020
- #869 "Exclude all files except…" | 653 | 2015
- #75559 Read files inside zip archives | 644 | 2019
- #43026 Ignore all whitespace in diff | 635 | 2018
- #5953 Print editor contents | 626 | 2016
- #179053 Built-in git graph visualization | 611 | 2023
- #32693 Monolithic repo, multiple project settings | 608 | 2017
- #3742 Auto-hide side bar | 597 | 2016
- #73780 More flexible decorations/editor overlays for extensions | 523 | 2019
- #1833 API for GUI customization by extensions | 470 | 2016
- #28795 Random scroll jumps (Electron/xinput) | 456 | 2017
- #41309 Configurable toolbar | 412 | 2018
— [GitHub open issues sorted by reactions](https://github.com/microsoft/vscode/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc)

Implemented (closed as completed, by 👍):
- #10121 Floating windows | 2,876 | closed 2023-12
- #396 Multi-root workspaces | 2,618 | 2017
- #10546 Terminal tabs | 2,411 | 2021
- #178 Git status in Explorer | 2,285 | 2017
- #37350 3-way merge editor | 1,464 | 2022
- **#21839 Private marketplace/gallery | 1,377 | closed 2025-11-19**
- #25887 Copy left/right in compare | 1,193 | 2022
- #5605 Symbols tree (Outline) | 1,062 | 2018
- #4865 Minimap | 1,056 | 2017
- #1023 Multi-select in file tree | 1,018 | 2018
- #12622 Pin tabs | 1,008 | 2020
- #14909 Grid editor layout | 1,000 | 2018
- #26777 Multiple views visible simultaneously | 871 | 2022
- #17777 Tree indent guides | 747 | 2019
- #2743 Settings Sync | 746 | 2020
- #6328 File nesting | 670 | 2022
- #23931 Search results in an editor tab | 634 | 2020
- #1012 Insert/overtype mode | 616 | 2024-11
- #211836 Indent with tabs default | 614 | 2024-06
- #8226 Select text from deleted lines in diff | 572 | closed 2025-10
- #180496 display prop in markdown spans | 607 | closed 2026-09
— [GitHub closed issues sorted by reactions](https://github.com/microsoft/vscode/issues?q=is%3Aissue+is%3Aclosed+sort%3Areactions-%2B1-desc)

Recent direction (2025–2026): Copilot Chat open-sourced (Jun 2025). The Oct 2026 issue tracker shows an "Agents window", worktree-based agent sessions, Copilot CLI agent host, MCP gallery/registry, a plugin/customizations marketplace, and Voice Mode. — [Visual Studio Magazine](https://visualstudiomagazine.com/articles/2025/06/30/vs-code-goes-transparent-as-open-source-ai-editor.aspx); [GitHub issues](https://github.com/microsoft/vscode/issues)

### Inferences
- Cheap wins a new editor could ship that VS Code has left unresolved for about 10 years:
  - Independent UI and editor font scaling.
  - Mouse-button bindings.
  - Macro recording.
  - Settings inheritance, per-platform settings, local uncommitted settings, and variables.
  - Grouped and vertical tabs.
  - A detachable terminal.
  - Tree-sitter highlighting (an obvious fit for Rust; Zed and Helix already use it).
  - A configurable status bar and toolbar.
  - A built-in git graph.
  - Ignore-whitespace diffs.
  - Printing.
- "Project-wide diagnostics" (#13953) is a performance and architecture problem. A fast native core could index the whole workspace.

### Gaps
- GitHub search caps; issues closed as "not planned" (for example, some Electron requests) were not reviewed.

## 5. Cursor and Windsurf (plus Trae): what people love and hate

### Takeaway
Cursor is the breakout fork: 17.9% usage, 19.3% among professionals. Users love its Tab next-edit prediction, codebase indexing, and agent. Users hate:
- The opaque switch from requests to compute pricing (June 2025), which led to refunds.
- Usage that runs out fast.
- A VS Code base that lags upstream for months, which breaks extensions.
- Losing Microsoft's proprietary extensions.

Windsurf was gutted by Google's $2.4B license-and-hire deal and sold to Cognition in July 2025. Trae drew a telemetry scandal.

### Cited Findings
**Cursor — positives**
- Tab predicts the next edit, the cursor location, and files to change, using recent edits, linter output, and project structure. Users report pressing Tab "hundreds of times per day", called the top reason people stay (vendor-adjacent, anecdotal). Full-repo semantic indexing and agentic workflows are core draws. — [morphllm](https://www.morphllm.com/comparisons/cursor-vs-vscode); [dev.to Cursor guide 2026](https://dev.to/stacknotice/cursor-ide-complete-guide-for-developers-2026-1gp1); [DeployHQ](https://www.deployhq.com/guides/cursor)
- Usage: 17.9% overall, 19.3% professionals (SO 2025). — [SO 2025](https://survey.stackoverflow.co/2025/technology/)

**Cursor — negatives**
- On 16 Jun 2025 Cursor moved Pro ($20) from request limits to compute limits and added a $200 Ultra plan. Users ran out after "a handful of prompts" and got unexpected overage charges. Cursor apologized and offered full refunds for 16 Jun–4 Jul. The revised Pro gives unlimited Tab and Auto plus $20 of frontier-model usage at API price. — [TechCrunch](https://techcrunch.com/2025/07/07/cursor-apologizes-for-unclear-pricing-changes-that-upset-users/); [Cursor forum](https://forum.cursor.com/t/clarifying-cursor-pro-pricing/113803); [Simon Willison](https://simonwillison.net/2025/Jul/5/cursor-clarifying-our-pricing)
- Upstream lag: after Cursor 3.0, the base was still VS Code 1.105.1 (Sept 2025). Staff said a "VS Code rebase isn't in the near-term plans" and that they are instead updating Cursor-published forks of Microsoft extensions. Recurring "incompatible with VS Code 1.9x" errors hit extensions such as Kubernetes and Remote-SSH, and users ask Cursor to bump to the 1.129 API. — [Cursor forum: outdated 1.105.1](https://forum.cursor.com/t/outdated-vscode-version-vscode-version-1-105-1/156543/4); [Cursor forum: stuck on 105](https://forum.cursor.com/t/why-cursor-is-still-stuck-on-vscode-base-105-its-like-5-months-old-base/164193/3); [Cursor forum: bump request](https://forum.cursor.com/t/request-please-bump-the-underlying-vs-code-extension-api-electron-version-to-the-latest-upstream-release/166421); [Kubernetes incompat](https://forum.cursor.com/t/cursor-is-incompatible-with-kubernetes-plugin-requires-vscode-version-update-to-1-98-0/76016)
- Marketplace: Cursor uses Open VSX, so some extensions (for example Pylance) are missing or differ. Microsoft's C/C++ is blocked in forks since Apr 2025. — [Cursor forum Pylance](https://forum.cursor.com/t/unable-to-find-pylance-in-the-extension-marketplace/105099/26); [The Register](https://www.theregister.com/2025/04/24/microsoft_vs_code_subtracts_cc_extension/)
- Cursor is a paying Open VSX commercial customer, alongside AWS and Google. — [Forrester OCX 2026](https://www.forrester.com/blogs/ocx-2026-open-source-as-strategy/)

**Windsurf**
- In July 2025 Google hired CEO Varun Mohan and co-founder Douglas Chen in a $2.4B licensing deal. Cognition (maker of Devin) then bought the remaining IP, brand, and team (~$85M ARR) and planned to integrate Devin into the IDE. Outrage centered on employees left out of the Google payout. Cognition said all staff would participate financially. — [TechCrunch](https://www.techcrunch.com/2025/07/14/cognition-maker-of-the-ai-coding-agent-devin-acquires-windsurf/); [InfoWorld](https://www.infoworld.com/article/4023030/cognition-agrees-to-buy-whats-left-of-windsurf.html); [Maginative](https://www.maginative.com/article/cognition-acquires-windsurf-after-googles-talent-raid-left-it-gutted/)
- Usage: 4.9% (SO 2025). — [SO 2025](https://survey.stackoverflow.co/2025/technology/)

**Trae (ByteDance)**
- With telemetry disabled, Trae still made ~500 calls in ~7 minutes, sending up to 26 MB. Data included hardware info, persistent machine and user IDs, activity timings, and full file paths. An earlier Unit 221B analysis found remote feature changes and a permanent device ID. Reported RAM use was ~6.3x VS Code and ~3x Cursor. The reporter was temporarily blocked on Trae's Discord. — [The Register](https://www.theregister.com/2025/07/28/bytedance_trae_telemetry/); [SC World](https://www.scworld.com/brief/espionage-purportedly-facilitated-by-bytedance-ai-coding-tool); [TechRadar](https://www.techradar.com/pro/security/bytedance-ai-tool-caught-spying-on-users)
- Usage: 0.8% (SO 2025).

### Inferences
- Forking VS Code buys instant ecosystem compatibility but creates a permanent rebase tax, and Cursor is visibly losing that fight. Microsoft can also cut off proprietary extensions at will. A from-scratch editor avoids both but must offer its own LSP/DAP-based story.
- AI features users value: next-edit prediction (not just completion), whole-repo context, and agents with reviewable diffs. Pain points to avoid: opaque usage-based billing, lock-in to one model vendor, and telemetry that ignores opt-out. Bring-your-own-key/local models with transparent costs is a positioning opportunity.
- Being AI-native is now table stakes in this segment. Agentic CLIs (Claude Code 9.7%, plus its 28.6M-install VS Code extension) show many users want the editor to host external agents rather than embed one vendor's.

### Gaps
- No primary user survey on Windsurf satisfaction after the Cognition purchase was found.
- Cursor's current (Oct 2026) pricing and whether it has since rebased were not verified.

## 6. VSCodium / Open VSX, and extension security

### Takeaway
People choose VSCodium for telemetry-free, MIT-licensed binaries without Microsoft branding. VSCodium is at 6.2% usage. The cost is Open VSX: no Pylance, C/C++, Remote-SSH/Dev Containers, or Live Share, and a smaller catalog. Open VSX has grown into critical shared infrastructure for all forks. Both registries have serious malware problems, and the GlassWorm worm campaign hit Open VSX repeatedly from Oct 2025 to Apr 2026.

### Cited Findings
**VSCodium / Open VSX**
- VSCodium: 6.2% usage overall, 5.5% among professionals (SO 2025). — [SO 2025](https://survey.stackoverflow.co/2025/technology/)
- Microsoft marketplace terms limit use to Microsoft-branded builds. Remote Development, Live Share, and some GitHub integrations are unavailable on Open VSX. Remote-SSH does not work with VSCodium per the NixOS wiki. Workarounds: install a .vsix manually, ask the author to publish to Open VSX, or edit product.json, which package updates overwrite. — [ArchWiki](https://wiki.archlinux.org/title/Code); [NixOS wiki](https://wiki.nixos.org/w/index.php?diff=7208); [linuxcommandlibrary](https://linuxcommandlibrary.com/man/codium)
- Open VSX scale: >7,000 extensions from ~5,000 publishers. Downloads were >110M/month, maintained by just 3 engineers, with AWS funding (2025). A later headline reports >300M monthly downloads. — [DevOps.com](https://devops.com/aws-to-help-fund-open-vsx-registry-hosted-by-eclipse-foundation/); [TechDogs/GlobeNewswire](https://www.techdogs.com/tech-news/globe-newswire/open-vsx-registry-surpasses-300-million-monthly-downloads-as-industry-leaders-back-critical-developer-infrastructure)
- Open VSX business model (2026): free but rate-limited for open source, paid tiers with SLAs for commercial users (AWS, Google, Cursor are first customers). A managed service with a 99.95% SLA launched May 2026. Rate limiting targets high-volume automated traffic. Data stays in EU/Canada. — [Forrester](https://www.forrester.com/blogs/ocx-2026-open-source-as-strategy/); [ADTmag](https://adtmag.com/articles/2026/05/05/eclipse-foundation-launches-managed-open-vsx-service-as-extension-registry-use-grows.aspx)
- The Eclipse Foundation mandated pre-publish checks in Feb–Mar 2026 for namespace impersonation, name spoofing, leaked credentials, and malicious patterns. — [The Hacker News label](https://thehackernews.com/search/label/Eclipse%20Foundation)

**Malicious extensions**
- GlassWorm was first seen Oct 2025:
  - It uses Solana blockchain command-and-control and invisible-Unicode code hiding.
  - It steals npm, Open VSX, GitHub, and Git credentials and crypto wallets, and self-propagates.
  - Dec 2025: 24+ new extensions.
  - 2026: 72 extensions abused `extensionPack`/`extensionDependencies` to pull hidden loaders on update.
  - Apr 2026: 73 cloned extensions, 6 confirmed malicious, the rest "sleepers" awaiting a malicious update.
  — [Dark Reading](https://darkreading.com/application-security/fresh-glassworm-vs-code-extensions-supply-chain); [InfoWorld](https://www.infoworld.com/article/4145589/open-vsx-extensions-hijacked-glassworm-malware-spreads-via-dependency-abuse-2.html); [CyberInsider](https://cyberinsider.com/new-glassworm-supply-chain-wave-hits-open-vsx-with-72-extensions/); [The Hacker News Apr 2026](https://thehackernews.com/2026/04/researchers-uncover-73-fake-vs-code.html)
- Microsoft Marketplace:
  - "MaliciousCorgi" (Jan 2026): fake AI assistants "ChatGPT – 中文版" (~1.34M installs) and ChatMoss (~150k) sent user data to China.
  - Material Theme (3.9M installs) was flagged for malicious code via a dependency (Feb 2025).
  - A cryptojacking campaign (Apr 2025) included "Discord Rich Presence" with 189k installs.
  - Only ~1,800 of ~45,000 publishers are verified.
  — [TechRadar](https://www.techradar.com/pro/security/malicious-microsoft-vscode-ai-extensions-might-have-hit-over-1-5-million-users); [Koi Security](https://www.koi.security/blog/2-6-exposing-malicious-extensions-shocking-statistics-from-the-vs-code-marketplace); [BleepingComputer](https://www.bleepingcomputer.com/news/security/malicious-visual-studio-code-extensions-with-millions-of-installs-discovered)
- Proof of concept (2024, older): a typosquatted Dracula theme got into large organizations. — [Koi blog](https://www.koi.ai/blog/1-6-how-we-hacked-multi-billion-dollar-companies-in-30-minutes-using-a-fake-vscode-extension)

### Inferences
- VS Code extensions run with full user privileges and no sandbox, and the attacks shown above exploit exactly that. A new editor should design for:
  - Capability-scoped plugins, such as WASM sandboxes with declared permissions for filesystem, network, and process.
  - Signed publishers.
  - No silent dependency auto-install.
  - Update diffing and pinning.
  This is a credible differentiator, not just hygiene.
- Registry strategy: forks depend on Open VSX, which is now partly paid and rate-limited. A new editor with a non-VS-Code plugin API needs its own registry, or must reuse LSP/DAP/tree-sitter and treat language support as "servers, not extensions".

### Gaps
- Microsoft's own Marketplace malware-takedown statistics for 2025–2026 were not found.
- No quantified survey of why users pick VSCodium; the reasons above come from documentation and community wikis.
