# Native and lightweight editors: what users love, hate and want (data pulled 2026-10-30)

Rust Code status labels used throughout: HAS = in the brief's list of existing features; PARTIAL = a related capability is listed but does not fully cover the item; NOT LISTED = absent from the brief's list. NOT LISTED means the brief does not mention it, not that the code was checked. No Rust Code source was inspected.

Method caveat: GitHub, Hacker News and forum pages were read through a summarising fetch tool. Counts are as returned on the pull date. Paraphrased user opinions are anecdotes, not survey data.

## Zed: top requests, praise and complaints

### Takeaway
Zed has closed many checklist gaps in 2025-2026 (AI off switch, minimap, sticky scroll, split diff, git graph, 1.0 on three OSes). Its remaining open demand centres on smooth scrolling, test running, multi-window, extension UI, offline/consent control and editor basics. Rust Code's strongest openings against Zed are consent-gated installs, no telemetry, no AI, a UI-capable plugin API and a test explorer, though its plugin and test features still need depth or breadth to compete.

### Cited Findings

Status and shipped features:
- Zed 1.0 shipped 29 April 2026 on macOS, Windows and Linux; it was mainly bug fixes plus bookmarks and a "view commit" action — [The Register](https://www.theregister.com/2026/04/30/zed_team_releases_version_10/). DevClass gives May 6, which conflicts and may reflect its later coverage — [DevClass](https://www.devclass.com/software/2026/05/07/zed-team-releases-version-10-of-rust-built-editor/5234770).
- The team says 1.0 does not mean done or perfect — [Phoronix](https://phoronix.com/news/Zed-1.0-Released).
- The Hacker News "Zed 1.0" story drew 2,153 points and 690 comments — [HN Algolia](https://hn.algolia.com/api/v1/search?query=Zed%201.0&tags=story&numericFilters=created_at_i>1777000000&hitsPerPage=3).
- A `disable_ai` setting and an onboarding switch now exist — [Zed blog](https://zed.dev/blog/disable-ai-features). The Register reports developers praised it — [The Register](https://www.theregister.com/2026/04/30/zed_team_releases_version_10/).
- Telemetry has separate `diagnostics` and `metrics` controls — [Zed docs](https://zed.dev/docs/telemetry.html). AI data sharing is opt-in — [Zed docs](https://zed.dev/docs/ai/privacy-and-security).
- Minimap arrived in 0.187.4 (May 2025), off by default — [Zed release](https://zed.dev/releases/stable/0.187.4).
- Editor sticky scroll arrived in 0.213.0 (November 2025, as a preview release) — [Zed release](https://zed.dev/releases/preview/0.213.0).
- Git graph replaced file history in 1.1.2, with lazy loading, search and a commit panel; the same release added side-by-side file diffs — [Zed release](https://zed.dev/releases/preview/1.1.2).
- Split diff reached agent review in 1.1.5 and commit history in 1.6.3 — [1.1.5](https://zed.dev/releases/stable/1.1.5); [1.6.3](https://zed.dev/releases/stable/1.6.3).
- Parallel agents were added in April 2026, and Zed co-develops the Agent Client Protocol — [The Register](https://www.theregister.com/2026/04/30/zed_team_releases_version_10/).

Top open issues by 👍, with Rust Code status — all from [GitHub search API](https://api.github.com/search/issues?q=repo:zed-industries/zed+is:issue+is:open+sort:reactions-%2B1-desc&per_page=40) (2,277 open matches):

| # | Issue | 👍 | Comments | Rust Code |
|---|---|---:|---:|---|
| 4355 | Smooth scrolling | 896 | 30 | NOT LISTED |
| 5242 | Test runner integration | 891 | 53 | PARTIAL: Rust/Cargo only |
| 21538 | jj SCM support | 574 | 59 | NOT LISTED |
| 4930 | Easymotion for Vim mode | 472 | 39 | NOT LISTED (Vim mode HAS) |
| 9662 | Secondary windows, multi-monitor | 466 | 54 | PARTIAL: terminal pop-out only |
| 21208 | Webview via extensions | 464 | 28 | PARTIAL: declarative plugin UI, no webview listed |
| 14801 | Flash.nvim style search | 400 | 77 | NOT LISTED |
| 9459 | Open `.code-workspace` files | 352 | 50 | NOT LISTED |
| 7808 | Make Python experience wonderful | 324 | 31 | NOT LISTED: brief does not name the 7 servers |
| 12589 | Node/npm downloads without consent | 311 | 94 | HAS: consent-gated installs |
| 9721 | Difftastic diffs | 311 | 16 | NOT LISTED |
| 6722 | Drag tab to new window | 292 | 33 | NOT LISTED |
| 11889 | iPad support | 258 | 39 | NOT LISTED |
| 4867 | Per-file tab settings | 200 | 13 | NOT LISTED |
| 10647 | Configurable mouse bindings | 200 | 19 | NOT LISTED |
| 5117 | Traditional Find All References view | 189 | 26 | NOT LISTED: LSP HAS, this view is not stated |
| 5010 | Built-in settings sync | 181 | 9 | NOT LISTED |
| 18698 | TypeScript server slower than VS Code | 171 | 76 | Not assessable |
| 24878 | Offline support meta-issue | 168 | 12 | PARTIAL: no telemetry/AI, full offline not stated |
| 34813 | Git 3-way merge | 161 | 13 | NOT LISTED |
| 13300 | Per-project themes | 160 | 37 | NOT LISTED |
| 14473 | Kinetic scrolling on Wayland | 158 | 9 | NOT LISTED |
| 7121 | Disable unwanted panels completely | 156 | 29 | NOT LISTED |
| 6754 | `--offline` runtime mode | 156 | 10 | NOT LISTED |
| 7409 | Localization | 154 | 43 | NOT LISTED |
| 45295 | Line-by-line staging | 151 | 12 | PARTIAL: hunks HAS, line level not stated |
| 13259 | Sudo/admin on save | 149 | 18 | NOT LISTED |
| 12039 | iOS/Android port | 147 | 35 | NOT LISTED |
| 4681 | Auto-detect indentation | 135 | 10 | NOT LISTED |
| 31102 | RTL text input/rendering | 132 | 41 | NOT LISTED |
| 12406 | Quarto `.qmd` | 129 | 10 | NOT LISTED |
| 17455 | Undo tree | 127 | 10 | NOT LISTED: local history is related |
| 57982 | Branch merging | 125 | 14 | NOT LISTED: branches HAS, merge not stated |
| 15896 | Unified global search palette | 124 | 5 | PARTIAL: quick open and palette HAS |
| 8043 | GPUI overdraw | 122 | 19 | Not assessable |
| 59500 | Remote devcontainers | 122 | 11 | NOT LISTED |
| 59157 | PR-style review comments to agent | 122 | 11 | Not applicable: no AI |
| 4663 | Recent files palette | 120 | 51 | NOT LISTED |
| 10966 | Markdown opens in preview | 116 | 38 | NOT LISTED |
| 14203 | Call hierarchy panel | 113 | 19 | NOT LISTED |

Extensions:
- Zed has about 1,000 extensions against over 100,000 for VS Code, and immature extensions and language support are a common complaint — [The Register](https://www.theregister.com/2026/04/30/zed_team_releases_version_10/).
- Top installs seen: HTML 7.6M, Git Firefly 1.8M, TOML 1.5M, Java 1.4M, Catppuccin 1.2M, Dockerfile 1.2M, SQL 871k, PHP 860k, Vue 725k, Ruby 638k, Catppuccin Icons 623k, XML 524k, Material Icon Theme 515k, macOS Classic 508k, C# 482k — [Zed extensions](https://zed.dev/extensions).
- Zed's FAQ says extensions cover languages, debuggers, MCP servers and themes, with UI customization planned later — [GitHub discussion](https://github.com/zed-industries/zed/discussions/12621).
- The Visual Extension API RFC (April 2026) has 41 👍. A maintainer replied on 21 May 2026 that it is "not something we're likely to get to in the near future". DB viewers and JSONL preview were common use cases — [Discussion #53403](https://github.com/zed-industries/zed/discussions/53403).
- One developer built a Claude Code panel in a fork because the extension API lacks native UI — [DEV](https://dev.to/tunadev0/zed-is-amazing-i-hate-it-5aob).

Praise, from the 1.0 thread (first 100k of 364k characters only) — [HN #47949027](https://news.ycombinator.com/item?id=47949027):
- A user called it a batteries-included Sublime alternative where Python linters work without setup.
- One user reported about 900MB with nine projects open, plus smooth UI, low input latency and fast startup.
- Several users praised SSH remote development and the Vim/Helix modes.
- A user handled text files in Zed that crashed other editors.
- A separate commenter said Zed is "everything I wanted Sublime to be" — [The Register](https://www.theregister.com/2026/04/30/zed_team_releases_version_10/).

Complaints, from the same thread unless noted — [HN #47949027](https://news.ycombinator.com/item?id=47949027):
- Memory: one user measured about 2.1GB RSS with a PHP and a Node project, mostly language servers (tsserver about 600MB). Another found it very memory-hungry on Linux.
- Idle CPU: one user measured about 50% of one core idle and about 800 syscalls per second unfocused, against zero for Sublime.
- Consent: multiple users object to Node and other binaries being installed and run without asking.
- Configuration: users find it arcane and want presets or profiles. One had an unwanted linter injected into a Rails project.
- Diffs: one user still uses VS Code for merge conflicts because the diff view lacks character-level differences.
- Direction: one user left for Kate, wanting stronger LSP and debugging and feeling Zed had shifted to AI.
- Terms: some raised concerns about the ToS and source code; another noted the ToS applies to hosted services.
- The Register adds that Zed is "not really lightweight" despite native code — [The Register](https://www.theregister.com/2026/04/30/zed_team_releases_version_10/).

Earlier complaints:
- Font rendering, likely mainly on low-DPI monitors, blocks one HN commenter from switching. The comment's date is unknown and may predate 1.0 — [HN mirror](https://grivy.duckdns.org/threads?id=fau).
- In the "Sequoia backs Zed" thread (about 2025), users worried about VC monetisation, wanted to pay for the editor without AI, and reported poor Linux performance until GPU configuration was changed — [HN mirror](https://hn.svelte.dev/item/44961172).
- A Sublime user went back because Zed's install nudges were constant, calling Sublime fast and quiet. Another reported Zed using 20GB on a 1GB-file search — search summary of [HN mirror](https://hn.svelte.dev/item/45362425); the snippet was not read in context, so the thread attribution is unverified.

### Inferences
- "Quiet" is a measurable pitch: zero idle CPU, no prompts, no background downloads. Sublime sets that bar, and users notice when it is missed.
- Consent-gated installs answer a 311-👍, 94-comment grievance open since 2024. This is the sharpest direct contrast with Zed.
- A declarative plugin UI answers a need Zed has publicly deferred. It only wins if real plugins (DB viewer, log/JSONL viewer) exist; a webview is a separate, 464-👍 demand.
- Highest-value absent items for Rust Code: smooth scrolling, multi-window/tab drag, Find All References view, call hierarchy, line-level staging, three-way merge, auto-detect indentation, jump motions and recent-files palette.
- Language servers dominate the memory numbers users quote, so RSS claims should separate editor and server memory. Egui's immediate-mode repaint could undercut the idle pitch unless it sleeps when nothing changes; verify this on Rust Code itself.

### Gaps
- Results past the top 40 issues, and exact ranks at the page boundary, were not examined. Closed issues were excluded, so demand for shipped features is not counted.
- Reddit r/ZedEditor was not reached. The remaining 264k characters of the HN thread were not read.
- Telemetry default state was not confirmed. No current evidence was found for Windows-specific quality, debugger depth, image viewer or settings-UI status.
- Zed's complete extension total and ranking method come from secondary or page-order evidence only.

## Sublime Text: what keeps users, what drives them away, what packages reveal

### Takeaway
Users stay for feel, low resource use and buffer safety; they leave over slow visible progress, licensing friction, ageing plugins and third-party LSP. Package data shows the editor's gaps are filled by plugins that Rust Code mostly ships natively.

### Cited Findings
- Stay reasons: never losing unsaved files, column editing, macros, plugins, responsiveness and low resource use — [HN #47949027](https://news.ycombinator.com/item?id=47949027).
- One user tried Zed and VS Code for weeks and returned for ergonomics; another praised auto-reloading Python plugins — [HN #47949027](https://news.ycombinator.com/item?id=47949027).
- Leave reasons: lost tabs or pay prompts after upgrades, plugins untouched for five years, and LSP that ignores local project dependencies — [HN #47949027](https://news.ycombinator.com/item?id=47949027).
- Users dislike time-limited update licensing and lack of significant releases since ST4. The volunteer-run LSP plugin and stale plugins also draw complaints — [HN mirror](https://hn.svelte.dev/item/44961172).
- Forum answers treat LSP as a third-party plugin — [Sublime Forum](https://forum.sublimetext.com/t/do-you-have-plans-to-add-native-lsp-support-in-next-releases/45134).
- Build 4200 (May 2025) added a right-side sidebar and TOML/Zsh syntaxes — [Linuxiac](https://linuxiac.com/sublime-text-code-editor-update-4200-add-sidebar-flexibility/).
- One blog reports Build 4213 in September 2026 with Python 3.14 plugins and faster Git status. I could not confirm this with Sublime HQ — [BornCity](https://borncity.com/news/sublime-text-4-213-python-3-14-und-neue-git-performance/).

Package Control all-time installs. These are cumulative, so legacy packages rank high and current use is overstated:

| Package | Installs | Gap filled | Rust Code |
|---|---:|---|---|
| Emmet | 6.14M | HTML/CSS expansion | NOT LISTED |
| SideBarEnhancements | 2.95M | File operations | NOT LISTED: file-tree operations not stated |
| BracketHighlighter | 2.67M | Bracket/tag highlight | NOT LISTED |
| SublimeLinter | 2.34M | Linting | HAS: LSP diagnostics, Clippy |
| WakaTime | 1.86M | Time tracking | NOT LISTED: possible plugin, conflicts with privacy if bundled |
| SublimeCodeIntel | 1.86M | Code intelligence | HAS: LSP |
| AutoFileName | 1.63M | Path completion | NOT LISTED |
| ChineseLocalizations | 1.60M | UI language | NOT LISTED |
| Alignment | 1.51M | Align selections | NOT LISTED |
| ColorPicker / Color Highlighter | 1.50M / 1.26M | Color editing | NOT LISTED |
| SublimeREPL | 1.48M | REPLs | PARTIAL: PTY terminal |
| Pretty JSON | 1.40M | JSON format/query | NOT LISTED |
| ConvertToUTF8 | 1.38M | GBK/BIG5 encodings | NOT LISTED |
| HTML-CSS-JS Prettify | 1.37M | Formatting | PARTIAL: LSP may format; not stated |
| A File Icon | 1.35M | Icons | HAS |
| SFTP | 1.28M | Remote edit/sync | NOT LISTED |
| Git | 1.23M | Git | HAS |
| DocBlockr | 1.21M | Doc comments | NOT LISTED |

Sources: [Package Control popular](https://packagecontrol.io/browse/popular). Others: [GitGutter 917K](https://packagecontrol.io/packages/GitGutter) (HAS), [Terminus 457K](https://packagecontrol.io/packages/Terminus) (HAS), [LSP 219K](https://packagecontrol.io/packages/LSP) (HAS).

### Inferences
- Encoding support, alignment, JSON tools, localization and remote edit are plugin-validated demands that the brief does not cover.
- Rust Code should copy Sublime's restraint: instant start, silent idle, reliable hot exit, and no nag-driven state loss.
- A one-time-license grievance suggests Rust Code should state its funding and licensing early, since users also distrust VC monetisation of Zed.

### Gaps
- Package Control's stats page was not read, so 30-day installs were unavailable. Forum vote counts were also unavailable; Discourse threads gave no comparable ranking.
- The September 2026 Build 4213 claim conflicts with a report that 4200 was still stable on 18 August 2026; those may simply be different dates. Official changelog not checked.
- Goto Anything praise is well known but did not appear in sources read.

## Lapce and other editors: status and lessons

### Takeaway
Lapce is alive but slow and thinly released, not abandoned. Small native editors keep loyal users through focus; 2026 evidence also shows updater security and native LSP management are now table stakes.

### Cited Findings

Lapce (Rust, Floem UI):
- Releases: v0.4.5 on 5 September 2025, v0.4.6 on 21 January 2026, and a nightly on 9 October 2026 — [GitHub releases](https://api.github.com/repos/lapce/lapce/releases?per_page=6).
- Commits landed in clusters on 6 and 30 September 2026, including remote folder choosing and LSP document highlights — [GitHub commits](https://api.github.com/repos/lapce/lapce/commits?per_page=10).
- Only 14 PRs were merged since 1 January 2026 — [GitHub search](https://api.github.com/search/issues?q=repo:lapce/lapce+is:pr+is:merged+merged:%3E2026-01-01&per_page=1).
- There are 833 open issues. Top 👍: Kakoune/Helix model 136; Vim support 46; grammars 35; `.editorconfig` 33; folding 29; smooth scrolling 25; Jupyter 21; plugin docs 17; DPI scaling ignored 14; Markdown renderer 14; Git integration 13; keyboard layout detection 11; font not rendered 10; outline 9; run tests 9 — [GitHub search](https://api.github.com/search/issues?q=repo:lapce/lapce+is:issue+is:open+sort:reactions-%2B1-desc&per_page=20).
- Rust Code status: HAS Helix/Vim, tree-sitter, folding, outline, Git, Cargo tests. NOT LISTED: `.editorconfig`, smooth scrolling, Jupyter, Markdown renderer.

Others:
- Nova 14 (20 July 2026) added integrated language server management with system detection and one-click install. It also added C, C++, C#, Swift and Rust syntax and debugging, plus per-language display settings — [Nova releases](https://nova.app/releases/).
- Nova 13 added Open Quickly in Projects and Case Preserving Replace — [Nova releases](https://nova.app/releases/). One reviewer notes that many extensions languish, especially for TextMate — [Coyote Tracks](https://coyotetracks.org/blog/panic-nova/) (date not verified).
- Notepad++: state-linked attackers hijacked updates for selected users from June 2025 — [The Register](https://www.theregister.com/2026/02/02/notepad_plusplus_intrusion/). CISA later listed CVE-2025-15556 as exploited; this comes from a search summary and was not opened at primary source — [UMich advisory](https://safecomputing.umich.edu/security-alerts/its-ia-advisory-update-notepad-windows-systems-recently-exploited-vulnerability).
- Kate: 26.04.0 shipped 16 April 2026, with 26.04.3 on 2 July — [KDE Apps](https://apps.kde.org/kate/). Users on the Zed thread praise it, and one left Zed for it — [HN #47949027](https://news.ycombinator.com/item?id=47949027).
- BBEdit: 16.0 reached 16.0.2 by mid-July 2026 and requires macOS 14 — [Releasebot](https://releasebot.io/updates/bare-bones-software). A G2 review cites a learning curve and limited free mode — [G2](https://www.g2.com/products/bbedit/reviews?xs=1).
- Lite XL: latest tagged release seen is v2.1.8 from June 2025 — [GitHub releases](https://api.github.com/repos/lite-xl/lite-xl/releases?per_page=2).
- Pragtical: a Lite XL fork prioritising practicality over minimalism — [FreeBSD review](https://reviews-dev.freebsd.org/R11:27bc50357c5d6d5db39766c6a5f58349bd9cb03f).
- Micro: Go terminal editor with multi-cursor, rebindable keys and a single dependency-free binary; 2.0.15 in Alpine edge — [Alpine](https://pkgs.alpinelinux.org/package/edge/community/x86_64/micro); [mirror README](https://upd.dev/zyedidia/micro).
- CudaText: a FreeBSD port references 1.230.2 — [FreeBSD log](https://cgit-dev.FreeBSD.org/ports/log/editors/Makefile?showmsg=1).

### Inferences
- Lapce's issue data suggests a lesson: basics such as folding, Vim, DPI scaling, fonts and keyboard layouts can absorb a small team for years. Rust Code already passes several, but it should test DPI, IME and keyboard layouts deliberately, since egui is also custom-rendered.
- The cause of Lapce's pace (team size, funding, UI-framework work) is not established by these sources.
- Rust Code's signed plugin registry and consent-gated installs fit the post-Notepad++ climate. Signing the editor's own updater matters too; the brief does not state whether it is signed.
- Nova 14 shows paid native rivals adding the same managed LSP story, so "consent-gated install" must be paired with polish.

### Gaps
- GitHub API returned 403 for Pragtical, Geany, TextMate and Fresh after rate limiting, so their status, plus GPU-rendered newcomers beyond Zed and Lapce, is unverified.
- No 2025-2026 user-sentiment evidence was found for Geany, TextMate, CudaText, Lite XL or Micro, and little for Nova or Notepad++ beyond the items above. Their positive/negative inventories are therefore incomplete; no feature-request vote data was gathered for them.
- Kate 26.04 feature notes and Notepad++ release details were not read at primary source.

## Feature matrix across editors

### Takeaway
Only features with a source in this research are asserted below. Most listed comparison features are absent from Rust Code's brief, and minimap, sticky scroll, previews and split-window behaviours are the clearest visible gaps relative to Zed.

### Cited Findings

| Feature | Evidence from other editors | Rust Code |
|---|---|---|
| Minimap | Zed since May 2025, off by default; [performance later improved](https://zed.dev/releases/stable/0.196.5) | NOT LISTED |
| Sticky scroll | [Zed editor-level, 0.213.0](https://zed.dev/releases/preview/0.213.0); [project panel, 0.195.2](https://zed.dev/releases/stable/0.195.2) | NOT LISTED (breadcrumbs HAS) |
| Markdown preview | Zed has it, with [font and theme settings](https://zed.dev/releases/preview/1.1.2) (exact release unverified); [default-open request](https://api.github.com/search/issues?q=repo:zed-industries/zed+is:issue+is:open+sort:reactions-%2B1-desc&per_page=40) has 116 👍; Lapce renderer request has [14 👍](https://api.github.com/search/issues?q=repo:lapce/lapce+is:issue+is:open+sort:reactions-%2B1-desc&per_page=20) | NOT LISTED |
| Image preview | No current evidence | NOT LISTED |
| Hex editor | No current evidence | NOT LISTED |
| Column selection | [Sublime users cite it](https://news.ycombinator.com/item?id=47949027) | PARTIAL: multi-cursor HAS, column mode not stated |
| Macro recording | [Sublime users cite it](https://news.ycombinator.com/item?id=47949027) | NOT LISTED |
| Split views | [Kate has them](https://apps.kde.org/kate/); side-by-side diff is separate | NOT LISTED for editor panes |
| Secondary windows | [Zed request, 466 👍](https://api.github.com/search/issues?q=repo:zed-industries/zed+is:issue+is:open+sort:reactions-%2B1-desc&per_page=40) shows the gap | PARTIAL: terminal pop-out |
| Tab groups, workspace switching | No current evidence; `.code-workspace` request has 352 👍 at Zed | NOT LISTED |
| Large files | Mixed: Zed praised for files that crashed other editors, yet a [20GB search report](https://hn.svelte.dev/item/45362425) exists | HAS: rope; project-search memory not assessed |
| Remote editing | [Zed SSH widely praised](https://news.ycombinator.com/item?id=47949027); [Nova remote fixes in 14.1](https://nova.app/releases/); [Lapce remote folder choosing](https://api.github.com/repos/lapce/lapce/commits?per_page=10); [Sublime SFTP plugin, 1.28M](https://packagecontrol.io/browse/popular) | NOT LISTED |
| Hot exit | [Key Sublime stay reason; users ask Zed not to lose tabs](https://news.ycombinator.com/item?id=47949027) | HAS |
| Git graph | [Zed 1.1.2](https://zed.dev/releases/preview/1.1.2) | NOT LISTED |
| Split diff | [Zed 1.1.2 onward](https://zed.dev/releases/preview/1.1.2) | HAS |

### Inferences
- A sensible order is parity items that Zed users already expect (split panes if truly missing, minimap, sticky scroll, Markdown preview, smooth scrolling, recent files), followed by differentiators (offline guarantee, line staging, merge tool, plugin UI showcase).
- Remote editing is the largest structural gap: it is a common reason for adopting Zed and has widespread praise, but it needs protocol work rather than a UI addition.

### Gaps
- Precise support and quality for tab groups, image viewers, hex editing, large-file modes, macros and column selection across Sublime, Kate, Notepad++, BBEdit, Nova, CudaText, Geany and TextMate were not verified. These are widely believed to exist in several of those editors, but I found no source in this run, so they are not asserted.
- Rust Code's split panes, line staging, column selection and offline behaviour should be checked against its code, because the brief is silent rather than negative.
