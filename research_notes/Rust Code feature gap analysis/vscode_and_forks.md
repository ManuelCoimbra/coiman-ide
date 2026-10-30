# VS Code and its forks (2025-2026): what users love, hate, want, and don't want, mapped to Rust Code

Data pulled 2026-10-30 unless marked otherwise.

Status key, used throughout:
- **HAS**: on the feature list in the research brief.
- **PARTIAL**: the brief lists something adjacent that does not fully cover the item.
- **NO**: nothing on the brief's list covers it.
- **VERIFY**: the brief is silent and it could exist as a detail of a listed feature.

All statuses come from the brief only. The Rust Code source was not inspected.

## What do VS Code users praise most and complain about most in 2025-2026?

### Takeaway
VS Code still dominates through its extension ecosystem and reach (75.9% usage in the 2025 Stack Overflow survey). The freshest complaints are AI crowding out editor work, default-on UI and AI changes, Electron-related memory and stability problems, and extension supply-chain attacks. Rust Code already answers the telemetry, Electron, and AI-push complaints by design. Its largest weakness against VS Code is ecosystem breadth.

### Cited Findings
**Praise and adoption**
- 2025 usage, all respondents: VS Code 75.9%, Visual Studio 29%, IntelliJ IDEA 27.1%, Cursor 17.9%, Neovim 14%, Claude Code 9.7%, Zed 7.3%, Windsurf 4.9% — [Stack Overflow 2025](https://survey.stackoverflow.co/2025/technology). Rust Code: not applicable.
- VS Code has been the most-used IDE five years running, while AI tools draw interest from its existing users — [Visual Studio Magazine](https://visualstudiomagazine.com/articles/2025/08/01/stack-overflow-dev-survey-visual-studio-vs-code-hold-of-ai-ides-to-remain-on-top.aspx?p=1).
- Install volume is a concrete proxy for ecosystem value: Python 243M, Live Server 84M, Prettier 73M, GitLens 54M, ESLint 54M — [Marketplace API](https://marketplace.visualstudio.com/_apis/public/gallery/extensionquery). Rust Code status: PARTIAL, because it has a WASM plugin API but no comparable catalogue.

**AI crowding out the editor**
- The July 2026 v1.130 notes are dominated by agents and chat. A rough estimate from reading the page is about 80% of feature content (reader estimate, not a published figure). Non-AI items include terminal diff links and community fixes — [VS Code 1.130](https://code.visualstudio.com/updates/v1_130). Rust Code: HAS the opposite position (no AI yet).
- The Agents window reached Stable as a preview in v1.120 (May 2026). v1.129 added an experimental docked editor panel and a modern UI preview. v1.130 added a shared agent host process — [VS Code 1.130](https://code.visualstudio.com/updates/v1_130); [VS Code 1.129](https://code.visualstudio.com/updates/v1_129); [Visual Studio Magazine](https://visualstudiomagazine.com/articles/2026/07/16/vs-code-1129-introduces-agent-host-and-experimental-agents-window-editor.aspx).
- `chat.disableAIFeatures` exists, but a team member conceded that new AI features sometimes slip past it — [VS Code FAQ](https://code.visualstudio.com/docs/supporting/FAQ); [HN thread](https://hn.svelte.dev/item/46501220). Rust Code: HAS (no AI), and any future AI should sit behind one enforceable gate.
- A university course guide warns that many AI features are on by default in recent versions — [HMC CS133](https://www.cs.hmc.edu/~beth/courses/cs133/lab/guides/vscode/index.html).

**Copilot commit-credit incident (2026)**
- `git.addAICoAuthor` launched off by default in 1.110, then flipped to `all` on April 16, 2026. The trailer reportedly appeared even with `chat.disableAIFeatures` true, and after commit finalization. PR #313931 reverted the default — [The Register](https://www.theregister.com/2026/05/04/microsoft_reverses_ai_credit_grab/); [It's FOSS](https://itsfoss.com/news/vs-code-credits-copilot-for-human-work/).
- Sources conflict on the cause: a reviewer attributed the revert to a bug rather than backlash, while The Register frames it as a response to complaints — [The Register](https://www.theregister.com/2026/05/04/microsoft_reverses_ai_credit_grab/).
- Rust Code status: HAS the commit panel. Lesson: never alter a commit message without showing it first.

**UI churn: "Modern UI" (August 2026)**
- Top-commented issues in one week: #328918 "New Modern UI is uncomfortable" (13 comments, complaints include being enabled by default), #329024 "visually noisy and less usable" (8), and #328757 lag on v1.131 fixed by disabling Modern UI (7). Also #328909 "New uggly look by default" — [Weekly Project News, Aug 2026](https://buttondown.com/weekly-project-news/archive/weekly-github-report-for-vscode-august-04-2026/). These comment counts are small and cover a single week.
- VSCodium users have opened [#3038 "Upstream vscode UI redesign"](https://github.com/VSCodium/vscodium/issues/3038) with 8 upvotes.
- Rust Code status: PARTIAL (themes and custom title bar). A stable layout promise is not on the list.

**Performance, memory, stability**
- August 2026 reports include a terminal output buffering leak (#329843), sluggish Agents window scrolling, freezes on launch, and a main-process JavaScript error on Windows (#328705) — [Weekly Project News, Aug 2026](https://buttondown.com/weekly-project-news/archive/weekly-github-report-for-vscode-august-04-2026/). Rust Code: HAS native rope and PTY architecture, but no public benchmark was found, so the advantage is unproven.
- A July 2026 digest reports Copilot freezes plus high CPU and memory in large workspaces — [Weekly Project News, July 2026](https://buttondown.com/weekly-project-news/archive/weekly-github-report-for-vscode-july-13-2026-july/). Seen as a search summary only; the page was not opened.
- A March 2026 report describes renderer out-of-memory crashes after hours of use with the Claude Code extension, tied to Electron's per-process cap — [issue mirror](https://claudeissues.com/issue/35968-vs-code-extension-renderer-oom-crash-code-536870904-and-maxlistenersexceeded-mem).
- One blog claims 180MB idle RAM for Zed against 650MB for VS Code, and comparison posts headline a 180ms vs 2.4s cold start — [Gil Ricardo](https://www.gilricardo.com/blog/zed-editor-review-2026); [Tech Insider](https://tech-insider.org/?p=19695). Treat these as unverified single-source numbers. An HN commenter disputes that Zed is clearly faster — [HN](https://news.ycombinator.com/threads?id=tmdh).

**Telemetry**
- [#176269](https://github.com/microsoft/vscode/issues/176269) "Require user consent before sending any telemetry" has 1,005 upvotes and is still open. Rust Code: HAS.

**Extension security**
- GlassWorm was disclosed in October 2025 as the first self-propagating VS Code extension worm. Socket later reported at least 72 further malicious Open VSX extensions between Jan 31 and Mar 13, 2026, using `extensionPack` and `extensionDependencies`. Another 73 impersonation extensions followed by April 25, 2026 — [CSA note](https://labs.cloudsecurityalliance.org/research/csa-research-note-glassworm-open-vsx-transitive-dependency-a/); [CSA v2 note](https://labs.cloudsecurityalliance.org/research/csa-research-note-glassworm-v2-openvsx-ai-dev-supply-chain-2/). These are secondary sources, and the primary Socket reports were not read.
- A dropper reportedly enumerates all VS Code-compatible IDEs and silently installs a second stage into each. The Eclipse Foundation began mandatory pre-publish checks in March 2026, but attackers still use a lower-friction path — [CSA note](https://labs.cloudsecurityalliance.org/research/csa-research-note-glassworm-supply-chain-developer-toolchain/); [Aviatrix](https://aviatrix.ai/threat-research-center/researchers-uncover-73-fake-vs-code-extensions-delivering-glassworm-v2-malware).
- Rust Code status: HAS (WASM sandbox, capability permissions, signed registry, lockfile, update diffs).

### Inferences
- Benign-then-malicious updates and transitive dependency pulls are exactly what update diffs and a lockfile target. Rust Code should also show dependency additions as a separate consent step.
- Weekly-looking release spacing (1.129 mid-July, 1.130 July 22) plus default-on UI changes suggests a positioning line: slow, opt-in UI change.
- The "Electron is heavy" claim only sells if measured. Rust Code should publish reproducible startup, idle RAM, and large-file numbers against VS Code and Zed.

### Gaps
- Stack Overflow 2026 results: none found. The 2025 admired and desired figures could not be extracted.
- Reddit r/vscode was not sampled directly, so praise evidence is mostly behavioural (usage, installs) rather than quotes.
- The GitHub digests are automated, and their comment counts cover short windows.

## What do Cursor, Windsurf, Kiro, and other fork users love and hate?

### Takeaway
Users come for integrated agents, then get burned by pricing, memory growth, data handling, and dependence on Microsoft's closed extensions. Forks also carry strategic risk: Windsurf was rebranded and Void's repository is archived. Rust Code avoids all of this but has no agent experience yet. A local, bring-your-own-model design is the clearest opening.

### Cited Findings
**Cursor**
- Loved: one reviewer returned to Cursor because Composer could scaffold a feature across five files at once — [Kunal Ganglani](https://www.kunalganglani.com/blog/zed-vs-cursor-editor). Rust Code: NO (no AI).
- Hated, pricing: on June 16, 2025, Pro moved from 500 fast responses to $20 of usage at API rates, and some users incurred overages unknowingly. The CEO apologized and refunds were offered (2025) — [TechCrunch](https://techcrunch.com/2025/07/07/cursor-apologizes-for-unclear-pricing-changes-that-upset-users/). 2026 plans listed by one guide: Pro $20, Pro+ $60, Ultra $200, Teams $40 per user — [Jet Admin](https://www.jetadmin.io/blog/cursor-pricing-explained-plans-credit-system-and-real-costs-in-2026/). Rust Code: not applicable until AI ships.
- Hated, memory: a report says RAM climbs during codebase search and commit generation in 3.18 and later until hang or crash — [Cursor Forum](https://forum.cursor.com/t/severe-memory-leak-in-cursor-agent-codebase-search-on-windows-11-since-versions-after-3-18/171322). Staff call long-session memory use a known issue — [Cursor Forum](https://forum.cursor.com/t/performance-cursor-ide-causes-severe-system-lag-and-high-memory-usage/167059). Also a report of worsening freezes on 3.21.9 — [Cursor Forum](https://forum.cursor.com/t/cursor-ide-degradation/172153).
- Hated, fork lag and bugs: a report of laggy vim navigation lists Cursor 2.2.36 on VS Code 1.105.1 — [Cursor Forum](https://forum.cursor.com/t/vim-mode-navigation-is-very-laggy/138606). Rust Code: HAS native Vim and Helix modes.

**Fork extension limits**
- Microsoft's C/C++ extension began enforcing an environment check on April 3, 2025 (v1.24.5), though its terms had barred outside use since at least September 2020. Pylance reportedly behaved this way for years. Cursor was accused of reverse-proxying Microsoft's marketplace (2025) — [The Register](https://www.theregister.com/2025/04/24/microsoft_vs_code_subtracts_cc_extension/).
- Cursor shipped in-house C#, C++, Python, Remote SSH, Containers, and WSL extensions — [Cursor Forum](https://forum.cursor.com/t/new-in-house-extensions-c-c-ssh-devcontainers-wsl-python/94531). VSCodium [#2300](https://github.com/VSCodium/vscodium/issues/2300) has 47 upvotes and [#1265](https://github.com/VSCodium/vscodium/issues/1265) (WSL remote) has 22.
- Rust Code status: HAS the open-LSP route (clangd, pyright, csharp-ls), which avoids these licences. NO for Remote SSH, WSL, and Containers.

**Windsurf / Devin Desktop**
- Cognition signed the acquisition on July 14, 2025. The product became Devin Desktop by over-the-air update on June 2, 2026, and Cascade reportedly sunset July 1, 2026 — [Devin FAQ](https://docs.devin.ai/desktop/devin-desktop-faq); [Toolpick](https://www.toolpick.dev/blog/windsurf-is-now-devin-desktop); [Business Tech Navigator](https://businesstechnavigator.com/news/windsurf-devin-desktop-cascade-eol-2026). The Cascade date comes from third-party sources.

**Kiro: top open requests**
- From the [Kiro tracker](https://github.com/kirodotdev/Kiro/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc):
  - Devcontainers: 133 (Rust Code NO)
  - Telemetry export frequency: 93 (not applicable)
  - WSL: 92 (NO)
  - BYOM via Bedrock: 90 (NO)
  - Anthropic API key: 77 (NO)
  - Tiers above Pro: 74 (not applicable)
  - "Make Kiro a VSCode Extension": 70 (not applicable)
  - Basic and Standard plans: 66 (not applicable)
- Docs reportedly allow client-side telemetry with opt-out, and Free and individual content may be used for improvement unless opted out — [Zenn](https://zenn.dev/shirochan/articles/331d0ecac815ce); [RFP Wiki](https://www.rfp.wiki/artificial-intelligence/ai-code-assistants/kiro/windsurf). Secondary sources.

**Trae**
- A developer report counted about 500 calls in about 7 minutes, totalling up to 26MB, with telemetry off. ByteDance reworded the toggle so it only covers VS Code framework telemetry (July 2025) — [The Register](https://www.theregister.com/2025/07/28/bytedance_trae_telemetry/). Rust Code: HAS (no telemetry, no network without consent).
- Top requests: Linux version (17 and 14 votes), lifecycle hooks for agents (16), custom models via LiteLLM (14) — [Trae tracker](https://github.com/Trae-AI/Trae/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc). Rust Code: VERIFY for Linux builds.

**Void**
- The repository reports `archived: true`, 28,727 stars, last pushed 2026-06-02 — [GitHub API](https://api.github.com/repos/voideditor/void).
- Top requests: Linux support (32), codebase indexing and custom docs (22), MCP (18) — [Void tracker](https://github.com/voideditor/void/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc).

**Positron**
- Top requests: R Markdown params (45), comment reflow (21), SQL support (21), Data Explorer search (20), Flatpak or Snap (19), Colab extension (16) — [Positron tracker](https://github.com/posit-dev/positron/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc). Rust Code: NO for data science, notebooks, and SQL.

**VSCodium**
- Other active requests: XDG directories for `.vscode-oss` (9), Python extension issues (9) — [VSCodium tracker](https://github.com/VSCodium/vscodium/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc).

**Editor plus terminal agent pattern**
- One developer dropped Cursor for Zed in February 2026 and runs Claude Code in the terminal — [Neura](https://neura.market/directories/cursor/blog/devto-3274221). Zed added Terminal Threads in May 2026, per [Kunal Ganglani](https://www.kunalganglani.com/blog/zed-vs-cursor-editor). Rust Code: PARTIAL (PTY tabs and Git diff, but no agent session surface).

### Inferences
- The cheapest credible AI path is a good host for external CLI agents: terminal, reviewable diffs, worktrees, and consent prompts, with bring-your-own key or local model. This avoids resale pricing risk and matches Kiro and Trae's BYOM demand.
- If AI ships, publish a data flow view and a hard spend cap. Those target the Trae and Cursor failures directly.
- Devcontainers and WSL are repeatedly requested across forks. They are likely Rust Code's most important non-AI parity gaps.

### Gaps
- Cursor's own tracker query failed (`getcursor/cursor` returned a search validation error), so no Cursor vote counts are available.
- 2026 Cursor pricing sources conflict: a tracker reported no changes through March 2026 — [SaaS Price Pulse](https://www.saaspricepulse.com/blog/cursor-pricing-history). Windsurf rating aggregates could not be verified.
- r/cursor and r/ChatGPTCoding were not sampled directly. "Love" evidence for Windsurf, Kiro, Trae, and Positron is thin. No reliable praise was found for them beyond marketing, so none is reported.
- Attribution caution: the Zed RAM and startup figures, the Composer anecdote, Terminal Threads, and the Kiro data-use claim came from search summaries that did not map each claim to one URL. The linked page is the best match, not a confirmed one, so re-check before quoting.
- "Pending Cursor acquisition" appeared in search snippets but was not verified and should not be relied on.

## What are the top 40 extensions by installs, and what gap does each fill?

### Takeaway
Raw rankings are dominated by language stacks and their auto-installed companions. Gaps that recur are remote development, Docker, notebooks, more debug adapters, web preview, ESLint and Prettier, and PR review. Rust Code's installed feature list already covers GitLens's blame and diff basics, Error Lens, Vim, rainbow brackets, indent guides, and icons.

### Cited Findings
Source for all rows: [Marketplace API](https://marketplace.visualstudio.com/_apis/public/gallery/extensionquery), sorted by installs, fetched 2026-10-30. Format is installs in millions, then average rating with rating count. The "gap filled" column is my description.

| # | Extension | Installs | Rating | Gap filled | Rust Code |
|---|---|---|---|---|---|
| 1 | Python | 243.4 | 4.21 (632) | Python tooling | PARTIAL (pyright) |
| 2 | Pylance | 210.1 | 2.95 (286) | Python language server, closed | HAS alternative (pyright) |
| 3 | Python Debugger | 148.8 | 4.71 (17) | Python debugging | NO (CodeLLDB only) |
| 4 | Jupyter | 110.9 | 2.64 (371) | Notebooks | NO |
| 5 | C/C++ | 106.4 | 3.42 (605) | C++ IntelliSense, debug | HAS alternative (clangd, CodeLLDB) |
| 6 | Jupyter Keymap | 90.5 | 4.25 (4) | Notebook keys | NO |
| 7 | Jupyter Renderers | 89.7 | 2.92 (13) | Rich notebook output | NO |
| 8 | Live Server | 84.0 | 4.40 (519) | Local web preview with reload | NO |
| 9 | Copilot Chat | 79.1 | 2.78 (406) | AI chat, agents | NO |
| 10 | Copilot | 74.8 | 4.08 (1058) | AI completion | NO |
| 11 | Prettier | 72.6 | 3.54 (492) | JS/web formatter | PARTIAL (LSP format only) |
| 12 | Jupyter Cell Tags | 70.9 | 4.33 (3) | Notebook tags | NO |
| 13 | Jupyter Slide Show | 70.9 | 5.0 (2) | Notebook slides | NO |
| 14 | CMake Tools | 64.6 | 4.47 (78) | CMake build, targets | NO |
| 15 | Python Environments | 63.2 | 1.30 (161) | venv management | NO |
| 16 | IntelliCode | 62.6 | 3.90 (104) | AI-ranked completion | NO |
| 17 | C/C++ Themes | 61.1 | 3.47 (19) | Themes | HAS (imports) |
| 18 | C/C++ Pack | 59.2 | 4.64 (47) | Bundle | NO bundles (VERIFY) |
| 19 | Java (Red Hat) | 58.3 | 3.32 (187) | Java language support | HAS (jdtls) |
| 20 | Chinese language pack | 56.8 | 4.83 (81) | UI localization | NO (VERIFY) |
| 21 | GitLens | 53.7 | 3.37 (945) | Blame, history, graph | PARTIAL (blame, diff; no graph) |
| 22 | ESLint | 53.6 | 4.29 (251) | JS lint diagnostics | NO (not on LSP list) |
| 23 | Java Debugger | 53.5 | 3.89 (28) | Java debug | NO |
| 24 | Docker | 53.2 | 4.48 (107) | Containers | NO |
| 25 | Maven | 49.6 | 4.12 (17) | Java build | NO |
| 26 | Java Test Runner | 48.2 | 4.24 (21) | Java tests | NO (Cargo only) |
| 27 | Java Project Manager | 47.7 | 3.58 (12) | Java projects | NO |
| 28 | Java Pack | 47.3 | 3.72 (89) | Bundle | NO |
| 29 | IntelliCode API Examples | 47.1 | 3.76 (21) | Code examples | NO |
| 30 | CMake (twxs) | 46.8 | 3.04 (28) | CMake syntax | VERIFY |
| 31 | C# | 44.1 | 2.73 (678) | C# language support | HAS (csharp-ls) |
| 32 | Code Runner | 42.6 | 4.40 (286) | One-click run for any language | PARTIAL (Cargo only) |
| 33 | Dev Containers | 42.5 | 4.05 (60) | Container dev | NO |
| 34 | WSL | 41.1 | 4.79 (86) | WSL dev | NO |
| 35 | Remote - SSH | 38.4 | 3.59 (223) | Remote dev | NO |
| 36 | GitHub PRs | 37.2 | 3.93 (148) | PR review | NO |
| 37 | Remote-SSH Editing | 36.2 | 4.19 (21) | SSH config | NO |
| 38 | Material Icon Theme | 36.1 | 4.94 (400) | File icons | HAS |
| 39 | .NET Install Tool | 31.0 | 2.15 (105) | Runtime install | PARTIAL (consent installs) |
| 40 | Remote Explorer | 30.7 | 4.50 (12) | Remote targets UI | NO |

Further extensions, from the same API query, which are outside the top 40 but useful signals:

| Extension | Installs (M) | Rating | Gap filled | Rust Code |
|---|---|---|---|---|
| HTML CSS Support | 30.0 | 3.46 | HTML/CSS completion | VERIFY |
| YAML | 29.8 | 3.49 | Schema validation | NO |
| Gradle | 29.4 | 3.25 | Java build | NO |
| Claude Code | 29.1 | 3.71 (837) | Agent | NO |
| Auto Rename Tag | 26.2 | 3.68 | Paired tag edit | NO |
| Rainbow CSV | 24.8 | 4.79 | CSV columns | NO |
| Live Share | 24.4 | 4.24 | Collaboration | NO |
| WakaTime | 23.1 | 4.07 | Time tracking | NO |
| PowerShell | 22.5 | 3.56 | PowerShell language | NO |
| GitHub Theme | 20.5 | 4.59 | Theme | HAS (imports) |
| Go | 20.0 | 4.28 | Go tooling | PARTIAL (gopls) |
| Path Intellisense | 19.6 | 4.85 | Path completion | NO (VERIFY) |
| Code Spell Checker | 18.5 | 4.47 | Spelling | NO |
| Git History | 17.5 | 4.44 | File and line history | NO (VERIFY) |
| Git Graph | 15.6 | 4.93 (680) | Commit graph | NO |
| Markdown All in One | 14.9 | 4.69 | Markdown editing, preview | NO (VERIFY) |
| Tailwind CSS | 14.8 | 4.47 | Class completion | NO |
| PDF viewer | 14.3 | 4.47 | PDF in editor | NO |
| Indent Rainbow | 13.5 | 4.82 | Indent colours | PARTIAL (indent guides) |
| CodeLLDB | 11.1 | 4.93 | Native debug | HAS |
| Better Comments | 10.8 | 4.83 | Tagged comment colours | NO |
| Error Lens | 10.2 | 4.87 | Inline diagnostics | HAS |
| Vim | 9.2 | 4.09 | Vim mode | HAS |
| Todo Tree | 7.9 | 4.71 | TODO panel | NO |
| REST Client | 7.7 | 4.91 (388) | HTTP files | NO |
| Thunder Client | 7.7 | 2.36 (771) | API GUI | NO |
| Project Manager | 7.7 | 4.77 | Project switcher | NO (VERIFY) |
| Hex Editor | 7.5 | 3.51 | Binary editing | NO |
| SQLTools | 7.2 | 3.45 | Database client | NO |
| rust-analyzer | 6.8 | 4.80 | Rust language | HAS |
| Cline | 5.8 | 4.01 | Open agent | NO |
| Import Cost | 5.6 | 3.03 | Bundle size hints | NO |
| Bookmarks | 5.2 | 4.58 | Line bookmarks | NO (VERIFY) |
| Even Better TOML | 5.0 | 4.32 | TOML support | VERIFY |
| Peacock | 4.5 | 4.59 | Per-workspace colour | NO |
| Continue | 4.5 | 3.18 | Open agent | NO |
| Draw.io | 4.3 | 4.91 | Diagrams | NO |

**Open VSX, by downloads** ([Open VSX API](https://open-vsx.org/api/-/search?sortBy=downloadCount&sortOrder=desc&size=25), rating in brackets):
- Pyrefly 93.0M (4.5)
- Claude Code 63.0M (3.42)
- debugpy 63.0M
- Python 61.9M (3.88)
- Red Hat Java 47.4M
- Ruby LSP 44.6M
- Devsense PHP tools 42-43M
- Go 43.1M
- clangd 31.7M
- GitLens 18.7M (2.90)
- OpenAI ChatGPT 12.9M

### Inferences
- Install counts overstate deliberate choice. Companion packages such as Jupyter Keymap have 90.5M installs but only 4 ratings, and Slide Show has 70.9M with 2 ratings.
- High ratings on small tools (Git Graph 4.93, REST Client 4.91, Material Icon 4.94, Error Lens 4.87) mark tight features worth building natively.
- Low ratings on heavy Microsoft or AI extensions (Python Environments 1.30, .NET Install Tool 2.15, C# Dev Kit 2.32, Jupyter 2.64, Copilot Chat 2.78, Pylance 2.95) mark openings to do better. Ratings skew towards the unhappy, so don't read them as average sentiment.
- Open VSX's heavy Pyrefly, Ruby LSP, and PHP presence suggests more LSP presets (Ruby, PHP, Pyrefly) are cheap ecosystem wins.
- Suggested native priority:
  1. Remote SSH, WSL, and Dev Containers.
  2. Commit graph and history.
  3. ESLint, Prettier, and YAML presets.
  4. debugpy and other DAP adapters.
  5. Project-wide TODOs, bookmarks, spell check, and HTTP client.
  6. Web preview and Markdown preview.

### Gaps
- Rankings reflect cumulative installs, not active use. No public active-user data was found.
- Reasons behind low ratings were not researched. For example, the Thunder Client rating cause is unverified.
- Rust Code's "gap filled" labels are judgement calls, and VERIFY items need a codebase check.

## What are the top 30 most-upvoted open VS Code feature requests?

### Takeaway
The top of the list is mostly old requests about configurability rather than AI: fonts, mouse bindings, macros, settings scoping, tabs, and layout. Rust Code already satisfies several outright (tree-sitter, Vim, telemetry consent, terminal detach), and its native UI and TOML settings make many others cheap wins.

### Cited Findings
Source: [GitHub search API, open issues by +1](https://api.github.com/search/issues?q=repo:microsoft/vscode+is:issue+is:open&sort=reactions-%2B1&order=desc&per_page=45), fetched 2026-10-30. The requests date from 2015-2023.

| # | 👍 | Issue | Request | Rust Code |
|---|---|---|---|---|
| 1 | 3,796 | [#519](https://github.com/microsoft/vscode/issues/519) | Workbench font and size | VERIFY |
| 2 | 1,933 | [#3130](https://github.com/microsoft/vscode/issues/3130) | Custom mouse shortcuts | VERIFY, likely NO |
| 3 | 1,510 | [#13953](https://github.com/microsoft/vscode/issues/13953) | Project-wide JS/TS errors | PARTIAL (Cargo clippy only) |
| 4 | 1,370 | [#114851](https://github.com/microsoft/vscode/issues/114851) | Built-in Vim mode | HAS |
| 5 | 1,369 | [#15909](https://github.com/microsoft/vscode/issues/15909) | Settings files that extend others | PARTIAL (layered TOML) |
| 6 | 1,347 | [#70764](https://github.com/microsoft/vscode/issues/70764) | iPad version | NO |
| 7 | 1,228 | [#4490](https://github.com/microsoft/vscode/issues/4490) | Macro recording | NO |
| 8 | 1,152 | [#5595](https://github.com/microsoft/vscode/issues/5595) | Platform-scoped settings | VERIFY |
| 9 | 1,052 | [#54272](https://github.com/microsoft/vscode/issues/54272) | Markdown in diagnostics | VERIFY |
| 10 | 1,044 | [#40233](https://github.com/microsoft/vscode/issues/40233) | Local, uncommitted workspace settings | PARTIAL (layered TOML) |
| 11 | 1,005 | [#176269](https://github.com/microsoft/vscode/issues/176269) | Telemetry consent | HAS |
| 12 | 945 | [#43188](https://github.com/microsoft/vscode/issues/43188) | Multiple workspaces per window | VERIFY |
| 13 | 855 | [#100335](https://github.com/microsoft/vscode/issues/100335) | Chrome-like grouped tabs | NO (VERIFY) |
| 14 | 843 | [#40239](https://github.com/microsoft/vscode/issues/40239) | Enable or disable extensions by config | PARTIAL (lockfile) |
| 15 | 810 | [#34442](https://github.com/microsoft/vscode/issues/34442) | Extract terminal to a window | HAS |
| 16 | 742 | [#2809](https://github.com/microsoft/vscode/issues/2809) | Variables in settings | VERIFY |
| 17 | 739 | [#50140](https://github.com/microsoft/vscode/issues/50140) | Tree-sitter highlighting | HAS |
| 18 | 720 | [#35744](https://github.com/microsoft/vscode/issues/35744) | Movable status bar items | NO (VERIFY) |
| 19 | 709 | [#3884](https://github.com/microsoft/vscode/issues/3884) | Tidy app data folder structure | VERIFY |
| 20 | 699 | [#871](https://github.com/microsoft/vscode/issues/871) | Macro-like keybindings | NO (VERIFY) |
| 21 | 660 | [#108264](https://github.com/microsoft/vscode/issues/108264) | Vertical tabs | NO (VERIFY) |
| 22 | 653 | [#869](https://github.com/microsoft/vscode/issues/869) | "Exclude all except" file patterns | VERIFY |
| 23 | 645 | [#75559](https://github.com/microsoft/vscode/issues/75559) | Read files inside zip archives | NO |
| 24 | 635 | [#43026](https://github.com/microsoft/vscode/issues/43026) | Ignore all whitespace in diff | VERIFY |
| 25 | 626 | [#5953](https://github.com/microsoft/vscode/issues/5953) | Print editor contents | NO |
| 26 | 611 | [#179053](https://github.com/microsoft/vscode/issues/179053) | Git graph | NO |
| 27 | 608 | [#32693](https://github.com/microsoft/vscode/issues/32693) | Per-project settings in monorepos | PARTIAL (layered TOML) |
| 28 | 597 | [#3742](https://github.com/microsoft/vscode/issues/3742) | Auto-hide sidebar | NO (zen mode is adjacent) |
| 29 | 523 | [#73780](https://github.com/microsoft/vscode/issues/73780) | Flexible editor overlays for extensions | PARTIAL (declarative plugin UI) |
| 30 | 493 | [#115266](https://github.com/microsoft/vscode/issues/115266) | Minimap for notebooks | NO |

Next on the list:
- [#1833](https://github.com/microsoft/vscode/issues/1833): GUI customization API, 470. PARTIAL.
- [#45177](https://github.com/microsoft/vscode/issues/45177): individual files from other folders, 456. VERIFY.
- [#28795](https://github.com/microsoft/vscode/issues/28795): scroll jump bug tied to Electron and xinput, 457. Likely avoided, untested.
- [#26729](https://github.com/microsoft/vscode/issues/26729): shortcut learning hints, 429. NO.
- [#41309](https://github.com/microsoft/vscode/issues/41309): toolbar below menu, 412. NO.
- [#158603](https://github.com/microsoft/vscode/issues/158603): per-view sidebar widths, 412. VERIFY.
- [#5402](https://github.com/microsoft/vscode/issues/5402): improved column selection, 408. VERIFY.
- [#193267](https://github.com/microsoft/vscode/issues/193267): detach views and panels, 400. PARTIAL (terminal only).
- [#3429](https://github.com/microsoft/vscode/issues/3429): theme backgrounds, 380. NO.

### Inferences
- Best value-for-effort picks for Rust Code, since they need no ecosystem: UI font (#519), mouse bindings (#3130), macros (#4490, #871), settings `extends`, platform scoping, variables, and local overrides (#15909, #5595, #2809, #40233, #32693), plus grouped and vertical tabs (#100335, #108264), commit graph (#179053), print (#5953), and any panel detach (#193267).
- A TOML "one screen settings model" could close seven-plus long-open requests totalling about 6,500 upvotes: #15909, #5595, #40233, #40239, #2809, #32693, #3884 sum to 6,467.
- iPad (#70764) doesn't fit a native desktop egui focus and should be an explicit non-goal unless the market changes.
- Multi-workspace (#43188) is a deeper architecture choice and should be decided early.

### Gaps
- Issues may be partly solved in VS Code while still open, so these counts show demand rather than exact current absence.
- GitHub's search ordering is approximately by +1 (#28795 at 457 sits after #45177 at 456, for example). The top 30 was kept as returned.
- Cursor forum votes were not extracted because the forum's feature-request ranking wasn't fetched.

## What is missing in Rust Code that could be done better natively?

### Takeaway
Rust Code's differentiators are already real on privacy, sandboxed plugins, and native architecture. Remaining work splits into three tiers: parity gaps blocking adoption (remote, containers, broader languages and debuggers), cheap native wins from the top-voted list, and an optional local-first AI layer.

### Cited Findings
Each item below rests on evidence above; the "better natively" design ideas are inferences, listed in the next subsection.
- Remote demand: Remote-SSH 38.4M, WSL 41.1M, Dev Containers 42.5M installs; Kiro devcontainers 133 and WSL 92 votes — [Marketplace API](https://marketplace.visualstudio.com/_apis/public/gallery/extensionquery); [Kiro tracker](https://github.com/kirodotdev/Kiro/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc). Status: NO.
- Git visualization: VS Code [#179053](https://github.com/microsoft/vscode/issues/179053) has 611 upvotes; Git Graph has 15.6M installs at 4.93. Status: NO.
- Notebooks and data: Jupyter 110.9M at 2.64, and Positron's top requests centre on Data Explorer and SQL — [Positron tracker](https://github.com/posit-dev/positron/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc). Status: NO.
- AI hosting: Claude Code has 29.1M Marketplace and 63.0M Open VSX installs; Cline and Continue have 5.8M and 4.5M. Status: NO.
- Security: GlassWorm's mechanism is covered by Rust Code's model — [CSA note](https://labs.cloudsecurityalliance.org/research/csa-research-note-glassworm-open-vsx-transitive-dependency-a/). Status: HAS.

### Inferences
**Tier 1: adoption blockers**
1. Remote SSH, WSL, and Dev Containers through a small Rust server. Measure its footprint against VS Code's.
2. More LSP and DAP presets: ESLint, YAML, Ruby, PHP, debugpy, Java and C# debug.
3. Prettier and external-formatter runner.
4. Docker view.
5. Generic task runner (replaces Code Runner, and goes beyond Cargo).

**Tier 2: cheap native wins**
1. Commit graph, file and line history, PR review.
2. Macro recorder and mouse bindings.
3. Grouped and vertical tabs, detach for any panel, movable status items, auto-hide sidebar.
4. Settings `extends`, platform scoping, variables, local overrides, UI font.
5. TODO panel, bookmarks, spell check, Markdown preview, HTTP files, hex viewer, CSV columns, project switcher, zip browsing, print.

**Tier 3: differentiators**
1. Publish benchmarks. Without them, "native is faster" is an untested claim.
2. "Stable UI contract": no default-on redesigns, and no commit message edits without preview.
3. Plugin trust UI: show new transitive dependencies and permissions in update diffs. Migration from VS Code settings, keybindings, and snippets is the matching switching-cost reducer.
4. AI as an off-by-default module with one gate, a network and data ledger, bring-your-own key or local model, spend caps, and an external agent terminal with reviewable diffs. Keep the "no AI" build option.
5. Notebooks and collaboration are expensive and should wait until demand from Rust Code's real users is clear.

**Things users say they don't want, which Rust Code should avoid**
- AI features that bypass off switches.
- Silent commit edits.
- Default-on redesigns.
- Unclear usage billing.
- Telemetry toggles that don't cover everything.
- Marketplace lock-in.
- Forced extension updates pulling new dependencies.

### Gaps
- No user research exists on which of these gaps would actually move users to Rust Code.
- Effort estimates were not researched.
- VERIFY items need a check against the Rust Code source.
