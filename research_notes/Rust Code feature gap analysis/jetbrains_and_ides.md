# Full IDEs (JetBrains, Visual Studio, Xcode, Eclipse, Android Studio): loyalty drivers, complaints, and gaps for Rust Code

Research date: 2026-10-30. YouTrack vote counts and Marketplace downloads were pulled live from their public APIs on that date.

Rust Code status labels:
- HAS: listed in the assignment's "already has" list.
- NO: named in the assignment as not present.
- NOT LISTED: absent from the assignment's list. Verify against the code, because this research did not inspect the Rust Code repository.

Source limits:
- blog.jetbrains.com, intellij-support.jetbrains.com, and several jetbrains.com/help pages returned 403 or 404 to direct fetches. Claims from those pages come from search-engine summaries and are marked "(search summary)". Verify them before quoting.
- jetbrains.com/idea/features returned an empty page.
- Search summaries did not always tie each claim to one URL. For claims marked "(search summary)", the link is the best-matching result returned, not a confirmed page-level citation.

## Which RustRover features do Rust developers cite as reasons to choose it over VS Code + rust-analyzer?

### Takeaway
The durable reason is integration: one project model and engine across Cargo, tests, debugging, VCS, refactoring, HTTP, Docker, and databases. Public evidence for debugger superiority is weak and mixed, so a stable debugger and fast, accurate analysis are real openings for Rust Code.

### Cited Findings

Positioning and user reasons:
- RustRover uses its own analysis engine, not rust-analyzer, tied to an integrated project model covering Cargo, testing, debugging, and VCS. VS Code is modular and extension-driven (vendor, search summary) — [JetBrains, Oct 2026](https://blog.jetbrains.com/rust/2026/10/01/rustrover-vs-vs-code/)
- JetBrains argues project-wide refactoring is the main advantage (vendor, search summary) — [JetBrains, Oct 2026](https://blog.jetbrains.com/rust/2026/10/01/rustrover-vs-vs-code/)
- A user calls it "a true IDE, unlike VSCode", citing features integrated through menus, shortcuts, and GUI, plus macro expansion and type detection (2024, older) — [Rust forum](https://users.rust-lang.org/t/rust-rover-users-how-do-you-rate-it/119074)
- A user cites extra refactors such as moving a function to another file, and more flexible refactors than VS Code (2024, older) — [Rust forum](https://users.rust-lang.org/t/rust-rover-users-how-do-you-rate-it/119074)
- Reddit praise says "easy and reliable", though expensive. Other commenters say VS Code + rust-analyzer is much faster on larger projects (2025, search summary) — [r/rust "Which IDE?" mirror](https://redlib.hbubli.cc/r/rust/comments/1k1wlpi/which_ide/mnq45ov/?context=3)
- One commenter prefers CLion with the Rust plugin for embedded and FFI work (2025, search summary) — [r/rust mirror](https://reddit.birdcat.cafe/r/rust/comments/1k1wlpi/which_ide)
- Reviewers from 2024 and earlier judged the debuggers roughly similar, with RustRover slightly smoother. A newer user report calls RustRover's debugger unreliable, especially on Windows (search summary) — [Medium mirror](https://readmedium.com/vs-code-vs-rust-rover-the-debugging-experience-hacking-with-rust-3cf0fe57e624); [Rust forum](https://users.rust-lang.org/t/rust-rover-users-how-do-you-rate-it/119074)

Vendor-claimed feature inventory, with Rust Code status. All rows are sourced to the [RustRover product page](https://www.jetbrains.com/rust/) unless noted.

| RustRover feature | Rust Code status |
|---|---|
| Context-aware completion, on-the-fly analysis, quick-fixes | HAS through LSP and code actions. Depth depends on rust-analyzer |
| "Smart refactorings" | PARTIAL: LSP rename and code actions. Dedicated refactoring UI NOT LISTED |
| Live templates | NOT LISTED |
| Installs `rustup` from the IDE | NOT LISTED |
| Applies compiler-suggested fixes | Likely HAS through clippy and code actions. Verify batch-apply |
| `Cargo.toml` dependency management, TOML code insight | NOT LISTED beyond tree-sitter highlighting |
| Highlights inferred types, macros, `cfg` blocks, unsafe code | PARTIAL: tree-sitter highlighting and inline diagnostics. Semantic `cfg` and unsafe highlighting NOT LISTED |
| Run, debug, test, custom run configurations | HAS for Cargo. Custom-binary configs are unconfirmed |
| Profiling | NO |
| Dedicated Cargo tool window | PARTIAL: Cargo run configs. Dependency and feature tree NOT LISTED |
| HTTP Client | NO |
| Database tooling (from DataGrip) | NO |
| Docker tool | NOT LISTED |
| Web technologies (from WebStorm) | PARTIAL: ts-ls and tree-sitter |
| Real-time collaboration | NO |
| Shared project settings, including code style | NOT LISTED |
| Git commits and branches | HAS |
| GitHub/GitLab pull requests and code review | NOT LISTED |
| AI: Claude, GPT, Gemini, Grok, or local model; Junie, Claude Agent, Codex, others via ACP; next-edit suggestions | NO |

2026 RustRover additions, showing the moving target:

| Feature | Release | Rust Code status | Source |
|---|---|---|---|
| Call Hierarchy | 2026.1 | NOT LISTED | [ComponentSource](https://www.componentsource.com/product/rustrover/releases/2956781) |
| cargo-nextest integration | 2026.1 | HAS | [ComponentSource](https://www.componentsource.com/product/rustrover/releases/2956781) |
| LLDB 21 | 2026.1 | Depends on CodeLLDB | [ComponentSource](https://www.componentsource.com/product/rustrover/releases/2956781) |
| "Don't step into libs" | 2026.1.x | NOT LISTED | [ComponentSource](https://www.componentsource.com/product/rustrover/releases) |
| axum and reqwest route discovery, route-to-handler navigation | 2026.2 | NOT LISTED | [JetBrains](https://blog.jetbrains.com/rust/2026/07/22/whats-new-in-rustrover-2026-2/) (search summary) |
| Ferrocene toolchain support | 2026.2 | NOT LISTED | same |
| Interactive declarative macro tester, showing input-to-expansion correspondence | 2026.2 | NOT LISTED | same |
| Agent skills manager, importing Claude Code and Codex skills | 2026.2 | NO | same |
| Automatic handling of simple conflicts across a changeset | 2026.2 | NO | same |
| Git worktree management | 2026.2 | NOT LISTED | same |
| `.env` file support | 2026.2 | NOT LISTED | same |

### Inferences
- Rust Code already matches much of the Rust-specific core: Cargo, tests, nextest, clippy, debugger, and visualizers. The biggest Rust-specific catch-up items are macro tooling, refactoring UI, call hierarchy, the Cargo dependency and feature view, and framework awareness such as axum routes.
- RustRover's engine is its differentiator and its liability. Rust Code's rust-analyzer base avoids a separate engine that can disagree with the compiler, but it inherits rust-analyzer's refactoring limits.
- JetBrains is shipping features that were recent top requests (call hierarchy, nextest), so parity targets will keep moving.

### Gaps
- No source compared macro expansion, debugger quality, or refactoring depth head to head with current data.
- Current RustRover docs on macro expansion and refactoring returned 404 at the guessed URLs. Inline debugger values, crate-docs behavior, and the Rustfmt UI were not verified, so they are not claimed here.
- The Rust-specific refactoring list is unconfirmed.
- RustRover's share among Rust developers was not found. The Rust project's survey text gives no editor percentages, because those appear only in charts.

## What do RustRover and IntelliJ users complain about most, and what do vote counts show?

### Takeaway
Live vote data points at embedded debugging for RustRover and at workflow gaps for the platform. Complaints from forums and support threads add false positives, speed, memory, AI credits, forced UI change, and subscription cost.

### Cited Findings

Top unresolved RUST issues. All rows come from the [YouTrack API](https://youtrack.jetbrains.com/api/issues?query=project:%20RUST%20State:%20Unresolved%20sort%20by:%20votes%20desc&fields=idReadable,summary,votes&$top=30), retrieved 2026-10-30.

| Issue | Votes | Request | Rust Code status |
|---|---|---|---|
| RUST-12476 | 88 | probe-rs debugging | NOT LISTED |
| RUST-12499 | 41 | Embedded debugging | NOT LISTED |
| RUST-11880 | 34 | Embedded GDB server config | NOT LISTED |
| RUST-9329 | 32 | Show type size and alignment | NOT LISTED |
| RUST-3730 | 30 | Bazel projects | NOT LISTED |
| RUST-2748 | 28 | STM32 via OpenOCD | NOT LISTED |
| RUST-4577 | 26 | Exclude tests from Find Usages and Find in Files | NOT LISTED |
| RUST-15247 | 20 | `cfg(not(test))` context switching | NOT LISTED |
| RUST-7514 | 18 | Go-to on `Into::into` lands on `From::from` | NOT LISTED |
| RUST-13198 | 17 | Run config for custom binaries | NOT LISTED |
| RUST-2437 | 15 | Execute scratch files | NOT LISTED |
| RUST-13614 | 14 | Completion fails after proto regeneration | Verify with rust-analyzer |
| RUST-11857 | 14 | Leptos RSX macro completion | Verify with rust-analyzer |
| RUST-10212 | 12 | cargo-script dependencies | NOT LISTED |
| RUST-7198 | 11 | Assembly view panel | NOT LISTED |
| RUST-17404 | 8 | Debugger renders via `Debug` or `Display` | PARTIAL: Rust visualizers |

Top RUST issues marked resolved, which show demand. The resolved flag does not prove a shipped feature. All rows come from the [YouTrack API](https://youtrack.jetbrains.com/api/issues?query=project:%20RUST%20sort%20by:%20votes%20desc&fields=idReadable,summary,votes,resolved&$top=25).

| Issue | Votes | Request |
|---|---|---|
| RUST-14452 | 105 | Web technology support |
| RUST-12006 | 66 | Remote debug with bundled gdb/lldb |
| RUST-12222 | 50 | Attach debugger to a running process |
| RUST-15839 | 48 | Strings and vecs not shown in debugger |
| RUST-7323 | 42 | Call hierarchy |
| RUST-12459 | 37 | cargo nextest |
| RUST-13368 | 35 | Running a test rebuilds everything twice |
| RUST-12355 | 29 | Criterion benchmark run configs |
| RUST-12206 | 21 | Debug rstest parameterized tests |
| RUST-11763 | 18 | Analysis stuck on p256 crate |

Top unresolved IJPL (IntelliJ Platform) issues. All rows come from the [YouTrack API](https://youtrack.jetbrains.com/api/issues?query=project:%20IJPL%20State:%20Unresolved%20sort%20by:%20votes%20desc&fields=idReadable,summary,votes&$top=30).

| Issue | Votes | Request | Rust Code status |
|---|---|---|---|
| IJPL-72504 | 702 | Standalone Git client | NOT LISTED |
| IJPL-54496 | 529 | National keyboard layouts | NOT LISTED |
| IJPL-77243 | 494 | Git flow GUI | NOT LISTED |
| IJPL-72762 | 415 | Bitbucket integration | NOT LISTED |
| IJPL-43713 | 370 | Per-project window modality | Verify |
| IJPL-531 | 349 | Command-line run configuration | NOT LISTED |
| IJPL-338 | 328 | Redesign file chooser | Verify |
| IJPL-8382 | 313 | Store project metadata outside project folder | Verify |
| IJPL-6073 | 308 | Per-project plugin lists | NOT LISTED |
| IJPL-73102 | 281 | VCS info in Project View | Verify |
| IJPL-8661 | 280 | Global settings for existing projects | Verify |
| IJPL-35295 | 262 | URL protocol handler to open files | NOT LISTED |
| IJPL-103639 | 260 | Terminal links to project sources | Verify |
| IJPL-184486 | 247 | Filter or grep console output | NOT LISTED |
| IJPL-177161 | 242 | Modal commit moved to a plugin in 2025.1 | Discussion thread. Rust Code has a commit panel |
| IJPL-81447 | 233 | Patch file support in editor | NOT LISTED |
| IJPL-222323 | 223 | Non-removable "Start Free Trial" button | HAS the advantage: no commercial nag listed |
| IJPL-37162 | 211 | Project-specific live templates | NOT LISTED |
| IJPL-84839 | 204 | `git log -S` and `-G` in Find Commit | NOT LISTED |
| IJPL-101423 | 204 | Standalone terminal | NOT LISTED |
| IJPL-99530 | 196 | Non-modal merge UI | NO: merge tool absent |
| IJPL-59208 | 196 | Editor Groups | NOT LISTED |
| IJPL-13075 | 180 | Settings Sync with remote dev | NO |
| IJPL-218396 | 177 | Docker on the remote machine | NO |
| IJPL-35265 | 172 | Flatpak package | Verify |

Top unresolved IDEA issues, mostly long-standing. All rows come from the [YouTrack API](https://youtrack.jetbrains.com/api/issues?query=project:%20IDEA%20State:%20Unresolved%20sort%20by:%20votes%20desc&fields=idReadable,summary,votes&$top=30).

| Issue | Votes | Request | Rust Code status |
|---|---|---|---|
| IDEA-68854 | 426 | Eclipse-like Problems view | Inline diagnostics HAS. Project-wide panel NOT LISTED |
| IDEA-15540 | 152 | Export breakpoints | NOT LISTED |
| IDEA-252110 | 136 | Remote profiling | NO |
| IDEA-134919 | 99 | Save debug variables and watches for later inspection | NOT LISTED |
| IDEA-83442 | 71 | Bytecode debugging | NOT LISTED; assembly-level analogue is unconfirmed |
| IDEA-152972 | 50 | Debug multiple processes in one session | NOT LISTED |
| IDEA-191180 | 50 | Custom filtering of frames at breakpoints | NOT LISTED |
| IDEA-267874 | 50 | Very slow Gradle builds on WSL | Not applicable; the WSL performance theme is relevant |

Accuracy and performance:
- JetBrains says reducing false positives is one of the most common user requests. It attributes them to mismatch between its engine and compiler behavior (search summary) — [JetBrains, June 2026](https://blog.jetbrains.com/rust/2026/06/09/fewer-false-positives-rustrover/)
- A third-party summary reports a reduction of up to 25% in 2026.2. Treat this as a vendor claim — [Feedbagel](https://feedbagel.com/post/fewer-false-positives-in-rustrover-20262the-rustrover-blog)
- False-positive issues appear among top RUST items, including RUST-19863 (12 votes, unresolved) and RUST-16902 and RUST-11921 (17 and 18 votes, resolved) — [YouTrack API](https://youtrack.jetbrains.com/api/issues?query=project:%20RUST%20sort%20by:%20votes%20desc&fields=idReadable,summary,votes,resolved&$top=25)
- Reddit users say error highlighting lagged after edits because Clippy ran first (2025, search summary) — [r/rust mirror](https://redlib.hbubli.cc/r/rust/comments/1k1wlpi/which_ide/mnq38h6/?context=3)
- Forum users report extract-function output that does not compile, "alpha" Inspect Code, invisible variables while stepping, hard-to-read `Option<T>` on Windows, missed breakpoints, and occasional need to clear caches (2024, older) — [Rust forum](https://users.rust-lang.org/t/rust-rover-users-how-do-you-rate-it/119074)
- G2 lists high memory use, slow startup, and slow indexing among frequent IntelliJ review themes (anecdotal) — [G2](https://www.g2.com/products/intellij-idea/pricing)
- A 30 GB RAM user reports higher memory use in 2026.1.4 than 2025.3.2 on large Gradle projects with active indexing — [JetBrains support](https://intellij-support.jetbrains.com/hc/en-us/community/posts/37358146543122)

Pricing and licensing:
- RustRover is free for individual non-commercial use, and commercial work needs a subscription. Annual subscriptions include a perpetual fallback license for the purchased version — [RustRover](https://www.jetbrains.com/rust/)
- Commercial individual use was quoted at $69 per year in 2024 (older; current price not verified) — [Rust forum](https://users.rust-lang.org/t/rust-rover-users-how-do-you-rate-it/119074)
- On the non-commercial tier, detailed code-related data collection is on by default, including edit history, terminal usage, and AI interactions. It can be turned off. On commercial plans it is off by default — [RustRover](https://www.jetbrains.com/rust/); [RubyMine post](https://blog.jetbrains.com/ruby/2025/09/rubymine-is-now-free-for-non-commercial-use/) (search summary)
- The 2024 launch said anonymous statistics could not be disabled on the free tier. Current terms for that specific point were not confirmed — [JetBrains, 2024](https://blog.jetbrains.com/rust/2024/05/21/rustrover-is-released-and-includes-a-free-non-commercial-option/)
- A Capterra reviewer says pricing limits access for freelancers, students, and small businesses (anecdotal) — [Capterra](https://www.capterra.com/p/136010/IntelliJ-IDEA/)
- IntelliJ IDEA Ultimate and Community merged into one distribution, about 30% smaller than Ultimate alone (search summary) — [JetBrains, Dec 2025](https://blog.jetbrains.com/idea/2025/12/what-s-fixed-in-intellij-idea-2025-3/)

AI quotas and bundling:
- From August 25, 2025, monthly AI credits were tied to plan price, which shrank some quotas. AI Assistant and Junie share credits (search summary) — [JetBrains](https://blog.jetbrains.com/ai/2025/08/a-simpler-more-transparent-model-for-ai-quotas/)
- Users report fast credit burn and a tracker that does not show which queries cost most (search summaries) — [Support thread](https://intellij-support.jetbrains.com/hc/en-us/community/posts/31142845556370); [Junie credits thread](https://intellij-support.jetbrains.com/hc/en-us/community/posts/26179850596882/comments/26251610729490)
- JetBrains support said fewer than 1% of users hit the limit, and it points to local models as unaffected by quota (search summary) — [Support thread](https://intellij-support.jetbrains.com/hc/en-us/community/posts/31142845556370)
- Some Reddit users dislike JetBrains' AI focus (2025, search summary) — [r/rust mirror](https://reddit.birdcat.cafe/r/rust/comments/1k1wlpi/which_ide)

UI and product churn:
- Classic UI became a separate plugin from 2024.2. Complaints cite less clear icons, excess minimalism, and VS Code resemblance. More than 100,000 developers downloaded the plugin (search summary) — [DevClass](https://devclass.com/2024/11/18/jetbrains-releases-intellij-idea-2024-3-and-other-ides-still-struggles-to-convince-devs-of-benefits-of-new-ui); [Techzine](https://www.techzine.eu/news/devops/126504/jetbrains-users-struggle-with-transition-to-new-ui/)
- Sources conflict on the classic plugin's end: "at least July 2025" versus permanent phase-out in July 2025 (search summary) — [DevClass](https://devclass.com/2024/11/18/jetbrains-releases-intellij-idea-2024-3-and-other-ides-still-struggles-to-convince-devs-of-benefits-of-new-ui); [Techzine](https://www.techzine.eu/news/devops/126504/jetbrains-users-struggle-with-transition-to-new-ui/)
- Fleet ended on December 22, 2025, with downloads removed. Its platform moved to JetBrains Air, an agentic tool (secondary coverage) — [AlternativeTo](https://alternativeto.net/news/2025/12/jetbrains-will-discontinue-its-fleet-ide-on-december-22-2025-due-to-overlapping-purposes/); [DevClass](https://devclass.com/2025/12/09/jetbrains-abandons-fleet-ide-pins-hopes-on-forthcoming-air-agentic-development-tool/)
- Code With Me: 2026.1 is the last release with official support, and security updates run until sunset in Q1 2027. Remote development is said to be unaffected (search summary) — [JetBrains, March 2026](https://blog.jetbrains.com/platform/2026/03/sunsetting-code-with-me/)
- In the 2025 Rust project survey (9,389 participants per secondary search summary), Zed "did a remarkable jump", Helix was second, and agentic editors appear to be "eroding the userbase of VSCode and IntelliJ" — [Rust Blog, 2026](https://blog.rust-lang.org/2026/03/02/2025-State-Of-Rust-Survey-results)
- The same survey says the debugging story "slipped from 2nd to 4th place", and resource usage remains high — [Rust Blog, 2026](https://blog.rust-lang.org/2026/03/02/2025-State-Of-Rust-Survey-results)

### Inferences
- Rust vote totals are small compared with platform totals. Treat them as direction, not market size.
- Embedded debugging is the clearest unmet RustRover demand: RUST-12476, RUST-12499, RUST-11880, and RUST-2748 total 191 votes. Votes may overlap across issues.
- Low-cost RUST wins include type size and alignment display, excluding test code from search, and a custom-binary run config.
- The top platform request (702 votes) wants Git as a focused tool. Rust Code's Git panel plus 3-way merge and interactive rebase could target this directly.
- JetBrains' removals (Fleet, Code With Me, classic UI) create a stability message for Rust Code: a predictable UI, local-first operation, and no credit meter. Collaboration needs caution, because JetBrains is sunsetting its own tool for reasons not verified here.

### Gaps
- Reddit and Hacker News 2026 threads were not directly retrievable. Sentiment beyond the sources above is unmeasured.
- The Developer Ecosystem 2025 RustRover share was not found.
- JetBrains' stated reason for sunsetting Code With Me was not read directly.
- YouTrack counts may exclude issues restricted from anonymous API access. The IDEA list is also dominated by old items, so it under-represents complaints filed in IJPL.

## Which IDE-grade features are loved, and does Rust Code have them?

### Takeaway
Rust Code's largest verified feature gaps are profiling, merge and rebase, HTTP, database, structural search, and hot reload. Visual Studio shows that profiling integrated into the debug loop is a stronger pattern than a separate profiler screen.

### Cited Findings

| Feature | Detail | Rust Code status | Source |
|---|---|---|---|
| Structural search and replace | Syntax-aware templates with conditions. IntelliJ's page lists Java, Kotlin, Scala, and Groovy, not Rust | NO | [JetBrains docs](https://www.jetbrains.com/help/idea/structural-search-and-replace.html) |
| LightEdit | Single-file editing without a project, opened by `-e`/`--edit`. No completion or navigation | Likely covered by a native editor's startup. Verify single-file launch | [JetBrains docs](https://www.jetbrains.com/help/idea/lightedit-mode.html) |
| Shelve and changelists | IDE patches; pick files or changelists; re-apply many times. Git stash takes all uncommitted changes | NOT LISTED | [JetBrains docs](https://www.jetbrains.com/help/idea/shelving-and-unshelving-changes.html) |
| Local history | Editing safety net | HAS | Assignment brief |
| VS Hot Reload | Works for .NET and native C++. Apply edits to a running app with Alt+F10, no breakpoint pause required. C++ needs `/ZI` and `/INCREMENTAL`; Debug configuration required for .NET | NOT LISTED; no Rust equivalent verified | [Microsoft Learn](https://learn.microsoft.com/en-us/visualstudio/debugger/hot-reload?view=vs-2022) |
| VS Diagnostic Tools window | CPU, memory, and PerfTips during debugging. Top Functions and Hot Path; heap snapshot diffs | NO | [Microsoft Learn](https://learn.microsoft.com/en-us/visualstudio/profiling/profiling-feature-tour?view=vs-2022) |
| VS Performance Profiler | CPU, memory, instrumentation, file I/O, GPU; WSL, Docker, and Linux targets | NO | [Microsoft Learn](https://learn.microsoft.com/en-us/visualstudio/profiling/profiling-feature-tour?view=vs-2022) |
| VS Copilot Profiler Agent | Agent-directed CPU profiling and AI PerfTips | NO | [Microsoft Learn](https://learn.microsoft.com/en-us/visualstudio/profiling/profiling-feature-tour?view=vs-2022) |
| VS Live Unit Testing | Enterprise-only and .NET-only; runs affected tests as you type | NOT LISTED; test explorer HAS | [Microsoft Learn](https://learn.microsoft.com/en-in/visualstudio/test/live-unit-testing-intro?view=vs-2022); [NDepend, 2026](https://blog.ndepend.com/visual-studio-enterprise-vs-professional/) |
| VS IntelliTrace | Enterprise-only. Records exceptions, debugger events, and optionally function calls | NOT LISTED | [NDepend, 2026](https://blog.ndepend.com/visual-studio-enterprise-vs-professional/); [VS compare](https://visualstudio.microsoft.com/vs/compare/) |
| VS 2026 Coverage display | Coverage is reported among included tooling in secondary coverage | NO | [Syncfusion](https://www.syncfusion.com/blogs/post/whats-new-in-visual-studio-2026) |
| Xcode Instruments | An indie developer reportedly traced a leak in 10 minutes (search summary; exact page unconfirmed) | NO | [Search result candidate](https://www.screamingatmyscreen.com/apples-developer-experience-in-2026/) |
| RustRover test-to-source links, saved run configs | Test failure navigation | HAS (test explorer); verify link depth | [RustRover](https://www.jetbrains.com/rust/) |
| Assembly view | RUST-7198, 11 votes | NOT LISTED | [YouTrack API](https://youtrack.jetbrains.com/api/issues?query=project:%20RUST%20State:%20Unresolved%20sort%20by:%20votes%20desc&fields=idReadable,summary,votes&$top=30) |
| Attach to process, remote debug | RUST-12222 (50) and RUST-12006 (66), both resolved | NOT LISTED | [YouTrack API](https://youtrack.jetbrains.com/api/issues?query=project:%20RUST%20sort%20by:%20votes%20desc&fields=idReadable,summary,votes,resolved&$top=25) |

Complaints about the other IDEs:
- Xcode 26 Instruments users report crashes when profiling with the SwiftUI instrument — [Apple Developer Forums](https://developer.apple.com/forums/thread/810574)
- One Apple forum poster says Project Navigator SCM status matches Git perhaps 50% of the time (search summary) — [Apple Developer Forums](https://origin-devforums.apple.com/forums/profile/szymczyk/replies)
- A blogger harshly criticizes Xcode 26, citing reproducible crashes (search summary; exact page unconfirmed) — [Search result candidate](https://www.screamingatmyscreen.com/apples-developer-experience-in-2026/)
- VS 2026 went GA on November 11, 2025, with built-in Copilot and Fluent UI. A secondary source claims 50% fewer UI hangs and 40% faster large-solution load; these are unverified against Microsoft — [TrustedTech](https://trustedtechteam.com/blogs/news/visual-studio-2026-everything-you-need-to-know)
- The VS C++ team reports 387 C++ issues fixed and 29 feature requests implemented over 12 months — [Microsoft C++ Blog](https://devblogs.microsoft.com/cppblog/whats-new-for-cpp-developers-in-visual-studio-2026-version-18-0)
- Capterra (March 30, 2026) rates Eclipse IDE 4.3 from 202 reviews and Android Studio 4.6 from 117 — [Capterra](https://www.capterra.com/compare/205298-246781/Eclipse-IDE-vs-Android-Studio)

### Inferences
- Ship profiling as a debug-loop feature first: elapsed time between stops, then CPU and memory capture with flame views. This copies the VS pattern and avoids a detached tool.
- Live testing and coverage gutters could leverage nextest for Rust. Microsoft limits its version to Enterprise and .NET, so a free Rust version would differentiate. Rust compile times are the main feasibility risk.
- A Rust structural search would exceed the documented IntelliJ list. Rust Code's tree-sitter base makes a syntax-only version plausible; verify semantic matching needs before scoping.
- General hot reload for Rust should not be promised from this evidence, because no Rust implementation was researched.

### Gaps
- Direct sources were not obtained for IntelliJ's refactoring list, inspection counts, intention actions, HTTP client, database tools, profiler, coverage, diagrams, interactive rebase, Gateway status, or 2026 merge tool behavior. These features are claimed here only where a citation above supports them. RustRover's page confirms HTTP Client, Docker, and databases.
- CodeLens current edition availability and sentiment were not confirmed; a search summary suggested it reached Community, but its source URL could not be pinned down. Rust Code status: NOT LISTED.
- Live Unit Testing and IntelliTrace sentiment evidence is thin and mostly dated. "Loved" is unproven beyond the documented feature value.
- Visual Studio Developer Community's current top-voted suggestions were not retrievable. The newest ranking found was from [2021](https://visualstudiomagazine.com/Articles/2021/04/21/top-vs-requests.aspx).
- No 2026 evidence was found for Android Studio Gemini agent complaints, Eclipse feature loyalty, or PyCharm-specific and CLion-specific reasons. Only the Capterra ratings above and one CLion embedded preference were found.

## What are the top JetBrains Marketplace plugins, and what gaps do they fill?

### Takeaway
Plugin downloads show demand for AI, Vim, visual readability, ignore files, Git insight, CSV editing, static analysis, and shortcut learning. Rust Code already has Vim modes and a plugin API, but the remaining convenience items are candidates for built-ins or first-party plugins.

### Cited Findings
Downloads come from the Marketplace API, `https://plugins.jetbrains.com/api/plugins/{id}`, retrieved 2026-10-30. This is a chosen sample, not a full ranking, because the API's search endpoint rejected the query.

| Plugin | Downloads | Gap filled | Rust Code status | Source |
|---|---|---|---|---|
| GitHub Copilot | 56,417,000 | AI assistance | NO | [API](https://plugins.jetbrains.com/api/plugins/17718) |
| Rainbow Brackets | 25,145,281 | Bracket readability | NOT LISTED | [API](https://plugins.jetbrains.com/api/plugins/10080) |
| IdeaVim | 22,450,535 | Vim emulation | HAS built in | [API](https://plugins.jetbrains.com/api/plugins/164) |
| .ignore | 19,953,491 | Ignore-file support | PARTIAL at most; verify | [API](https://plugins.jetbrains.com/api/plugins/7495) |
| SonarQube for IDE | 16,115,008 | Static analysis | PARTIAL: clippy and LSP diagnostics | [API](https://plugins.jetbrains.com/api/plugins/7973) |
| CSV Editor | 13,689,512 | Table editing of CSV | NOT LISTED | [API](https://plugins.jetbrains.com/api/plugins/10037) |
| Translation | 10,900,694 | In-IDE translation | NOT LISTED | [API](https://plugins.jetbrains.com/api/plugins/8579) |
| GitToolBox | 10,695,065 | Ahead/behind counts and Git extras | PARTIAL: Git and blame HAS | [API](https://plugins.jetbrains.com/api/plugins/7499) |
| Atom Material Icons | 10,195,687 | File and IDE icons | NOT LISTED | [API](https://plugins.jetbrains.com/api/plugins/10044) |
| Key Promoter X | 8,259,993 | Learn shortcuts | NOT LISTED | [API](https://plugins.jetbrains.com/api/plugins/9792) |
| MavenHelper | 6,292,455 | Dependency conflict analysis | NOT LISTED; Cargo analogue is a dependency view | [API](https://plugins.jetbrains.com/api/plugins/7179) |
| Shell Script | 5,323,368 | Shell coding assistance | Verify | [API](https://plugins.jetbrains.com/api/plugins/13122) |
| StringManipulation | 5,078,964 | Case, sort, align, escape | NOT LISTED | [API](https://plugins.jetbrains.com/api/plugins/2162) |
| Database Navigator | 5,047,154 | Database tools | NO | [API](https://plugins.jetbrains.com/api/plugins/1800) |
| Grep Console | 3,376,398 | Filter and highlight console output | NOT LISTED | [API](https://plugins.jetbrains.com/api/plugins/7125) |
| Rust | 2,441,751 | RustRover features in other IDEs | Not applicable | [API](https://plugins.jetbrains.com/api/plugins/22407) |
| Rainbow CSV | 1,835,656 | Color CSV columns | NOT LISTED | [API](https://plugins.jetbrains.com/api/plugins/12896) |
| Save Actions | 1,318,021 | Format and clean on save | Verify format-on-save | [API](https://plugins.jetbrains.com/api/plugins/7642) |
| AceJump | 1,286,829 | Jump to visible position | NOT LISTED | [API](https://plugins.jetbrains.com/api/plugins/7086) |
| Indent Rainbow | 1,104,563 | Indent readability | NOT LISTED | [API](https://plugins.jetbrains.com/api/plugins/13308) |
| PMD | 528,690 | Static analysis | Not applicable to Rust | [API](https://plugins.jetbrains.com/api/plugins/1137) |
| Presentation Assistant | 484,554 | Show action shortcuts | NOT LISTED | [API](https://plugins.jetbrains.com/api/plugins/7345) |
| CognitiveComplexity | 284,392 | Live complexity metric | NOT LISTED | [API](https://plugins.jetbrains.com/api/plugins/12024) |

- JetBrains described Rainbow Brackets as a bracket-coloring tool on the Marketplace since 2017 — [JetBrains, 2023](https://blog.jetbrains.com/platform/2023/02/top-10-plugins-for-intellij-based-ides/)
- In JetBrains' 2023 recap, the most-downloaded plugins that year were the Chinese language pack (4.9M), Dart (2M), and Flutter (1.9M), so localization matters. This data is older — [JetBrains, 2024](https://blog.jetbrains.com/platform/2024/01/jetbrains-marketplace-highlights-of-2023-major-updates-community-news/)
- A user complained of fewer plugins than VS Code and no Rune highlighting in RustRover (2024, older) — [Rust forum](https://users.rust-lang.org/t/rust-rover-users-how-do-you-rate-it/119074)

### Inferences
- Cheap built-ins with evidence: rainbow brackets and indents, ignore-file support, string manipulation, CSV view, console grep, jump navigation, and a shortcut hint mode.
- Downloads are lifetime totals, so they favor older plugins. Installed-base and paid revenue are unknown.
- Rust Code's signed WASM registry can host long-tail items such as translation and icon packs. Starter plugins could prove the API.
- Localization may deserve attention, given the Chinese language pack's 2023 lead. This research did not check whether Rust Code supports it.

### Gaps
- A verified top-N Marketplace ranking was not obtained. Plugin quality complaints were not found in a citable, quantified form.
- Paid plugin revenue and active-install counts are not public in the data retrieved.
