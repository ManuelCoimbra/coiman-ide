# Developer sentiment: JetBrains IDEs, Fleet, and Visual Studio (heavyweight IDEs vs VS Code-style editors)

Research date: October 2026. Sources are mostly 2024-2026. Many sentiment items are anecdotal (forum and blog posts). They are flagged as such.

## What features make people pay for JetBrains, and which do VS Code users miss most?

### Takeaway
People pay for a deeply integrated, index-backed IDE. Its main parts are safe codebase-wide refactoring, navigation (Find Usages, Search Everywhere), a strong built-in Git/merge UI, a debugger and test runner integrated with saved run configurations, database tools, and local history, all "out of the box" without assembling extensions. Switchers back from VS Code most often cite refactoring, Git, debugging/run configs, and DB tools.

### Cited Findings
- Usage scale: Stack Overflow 2025 survey ranks VS Code 75.9%, Visual Studio 29%, Notepad++ 27.4%, IntelliJ IDEA 27.1%. Multi-select, so totals exceed 100% — [Stack Overflow Developer Survey 2025](https://survey.stackoverflow.co/2025/)
- In the same survey, AI editors entered: Cursor 17.9%, Claude Code about 10% (9.7% in another write-up) — [Visual Studio Magazine](https://visualstudiomagazine.com/articles/2025/08/01/stack-overflow-dev-survey-visual-studio-vs-code-hold-of-ai-ides-to-remain-on-top.aspx); [secondtalent](https://www.secondtalent.com/resources/most-used-ides/)
- IntelliJ dominates Java. Secondary sources say IntelliJ use among Java devs rose from 71% to 84%, and VS Code is about 31% among Java devs. These are secondary aggregations, not verified against primary data — [HackerNoon](https://hackernoon.com/lang/bs/the-ide-isnt-dead); [secondtalent](https://www.secondtalent.com/resources/most-used-ides/)
- JetBrains' own 2025 Developer Ecosystem survey had 24,534 respondents from 194 countries. 85% regularly use AI for coding, and 62% rely on at least one AI assistant, agent, or editor. JetBrains admits "some bias is likely present" toward JetBrains users, and the survey consistently shows higher JetBrains/Java/Kotlin usage than Stack Overflow's — [InfoWorld via scour](https://scour.ing/p/https://www.infoworld.com/article/4077352/85-of-developers-use-ai-regularly-jetbrains-survey.html); [The Register](https://www.theregister.com/2025/10/21/massive_jetbrains_dev_survey)
- Raw unweighted data in that survey shows Cursor respondents rising from 135 (2024) to more than 2,300 (2025) — [The Register](https://www.theregister.com/2025/10/21/massive_jetbrains_dev_survey)
- "Why I switched back from VS Code to IntelliJ" (Sep 14, 2025) lists what the author missed in VS Code:
  - Integrated Git tool window: branches, stashes, local changes, interactive rebase, cherry-pick, inline diff. VS Code Git felt "piecemeal".
  - Index-backed Go to Declaration/Implementation, Find Usages, Search Everywhere.
  - Built-in test runners, debuggers, profilers, and refactorings with no extensions.
  - Conditional breakpoints, log points, remote attach, debugging tests from the runner.
  - Saved, shareable run configurations (working dir, env vars, entry points).
  - Database console, data editing, schema inspection (Ultimate).
  - The author concedes VS Code is better for quick prototypes and light edits, because of startup speed and footprint.
  - Source: [mokkapps.de](https://mokkapps.de/blog/why-i-switched-back-from-vscode-to-intellij-idea)
- One user says IntelliJ refactorings "make dealing with large Java codebases a pleasure". Another says VS Code is "sorely lacking, even with all of the plugins", in codebase-wide refactoring. A Go switcher, by contrast, noticed no limitations after moving to VS Code — [users.rust-lang.org thread](https://users.rust-lang.org/t/anyone-here-go-intellij-vscode/84499); [r/golang mirror](https://red.127.pp.ua/r/golang/comments/1hbehrr/moving_back_to_vscode/m1lt6z4/?context=3); [yomotherboard Q&A](https://yomotherboard.com/question/is-vscode-a-good-choice-for-large-java-projects-compared-to-intellij/)
- Local history: JetBrains keeps timestamped versions of every file, with a diff viewer and restore. It can restore deleted files from a folder's history, independent of VCS — [JetBrains blog (KO)](https://blog.jetbrains.com/ko/2020/03/26/local-history-intellij-save-your-code-ko)
- In 2025.3, IntelliJ IDEA Community and Ultimate merged into one distribution. All former Community functionality stays free for commercial use. Newly free features include basic database tools, non-JVM language support, and Kubernetes integration. LSP support no longer requires a subscription. A lapsed Ultimate subscription falls back to the free feature set rather than locking the user out — [JetBrains blog Dec 2025](https://blog.jetbrains.com/idea/2025/12/intellij-idea-unified-release/); [JetBrains help](https://www.jetbrains.com/help/idea/2025.3/intellij-idea-single-distribution.html); [JetBrains blog Jul 2025](https://blog.jetbrains.com/idea/2025/07/intellij-idea-moves-to-the-unified-distribution/)

### Inferences
- Highest-value features for a lightweight Rust editor to offer natively are:
  - a first-class Git UI: a 3-way merge tool, interactive rebase, stash/branch view, and inline diff gutter;
  - local history, which is cheap to build and much loved;
  - shareable run/debug configurations, plus test-runner-to-debugger integration;
  - "Search Everywhere"-style unified search.
  These do not require a heavy semantic index.
- Deep refactoring and inspections depend on language servers. A VS Code-style editor inherits LSP limits, so it should expose the full LSP surface (code actions, rename, call/type hierarchy, inlay hints) with polished UX.
- JetBrains making DB tools and LSP free in the base IDE suggests these are now "table stakes". Paid differentiation is moving to advanced and enterprise features and AI.

### Gaps
- No primary 2025 JetBrains survey percentages for IDE market share were retrieved.
- No quantitative survey of "which JetBrains feature VS Code users miss most" was found. The evidence is anecdotal.
- YouTrack top-voted issues were not retrieved.

## Main complaints: memory/indexing, startup, cost, new UI, AI pricing/quotas, bugs, plugin breakage

### Takeaway
The recurring pain points are:
- indexing that blocks features and reruns on branch switches;
- resource hunger, which JetBrains itself admits;
- subscription price rises (11-30% in Oct 2025);
- the forced New UI, which some call a VS Code lookalike;
- opaque AI credit quotas.

### Cited Findings
- Indexing: in large repos, indexing is slow. Branch switches and pulls can trigger a reindex, and most IDE features are unavailable while it runs (VirtusLab case study) — [VirtusLab](https://virtuslab.com/success-stories/accelerating-intelli-j-indexing/)
- JetBrains acknowledges IntelliJ's breadth makes it "somewhat resource-hungry" and made faster project loading a high priority (Nov 2024 post; page returned 403 on fetch, so the claim is from the search snippet) — [JetBrains blog](https://blog.jetbrains.com/idea/2024/11/faster-time-to-code-in-intellij-idea/)
- One 2025 report (Clojure/Cursive users, IntelliJ 2024.3 on Apple Silicon) describes "multiple second gc pauses while typing" that were fixed by downgrading to 2024.1. Other users could not reproduce it — [clojurians log](https://clojurians-log.clojureverse.org/cursive/2025-02-02)
- The standard workaround is "Invalidate caches" and excluding build output directories from indexing — [JetBrains support forum](https://intellij-support.jetbrains.com/hc/en-us/community/posts/206151639/comments/206244325)
- Price: JetBrains raised subscription prices 11-30% on Oct 1, 2025, after three years of stable prices. The stated reason was inflation. IntelliJ IDEA commercial SRP went from $599 to $719 per user per year. Individual Ultimate went from EUR 169 to EUR 199 for year one. Prepay at old prices was allowed until Sep 30, 2025 — [heise](https://heise.de/-10506695); [ComponentSource](https://www.componentsource.com/news/2025/08/20/price-change-jetbrains-products)
- New UI:
  - It became default for all users in 2024.2 (July 2024). The Classic UI moved to an unbundled Marketplace plugin, supported "for at least one year", which does not work with Gateway/remote — [JetBrains blog](https://blog.jetbrains.com/blog/2024/07/08/the-new-ui-becomes-the-default-in-2024-2/)
  - Devs complained it is inferior and resembles VS Code. JetBrains reported 87% adoption of the new UI before it was default — [DevClass](https://devclass.com/2024/08/06/jetbrains-updates-ides-with-new-ui-by-default-now-a-vs-code-lookalike-some-devs-think-so/)
  - JetBrains later admitted onboarding "weren't always enough" for people adjusting during busy workdays (Oct 2024) — [JetBrains blog](https://blog.jetbrains.com/idea/2024/10/bridging-the-gap-between-the-classic-and-new-uis/)
  - Support-forum thread "Why do you want to ruin everything with new UI?" — [JetBrains support](https://intellij-support.jetbrains.com/hc/en-us/community/posts/8872880708370?page=3)
- AI pricing/quotas:
  - Since Aug 2025, AI Assistant and the Junie agent share one monthly credit pool. Top-up credits are valid 12 months, and Ultimate has a larger quota for agentic work — [JetBrains AI blog](https://blog.jetbrains.com/ai/2025/08/a-simpler-more-transparent-model-for-ai-quotas/)
  - Users complain the value of a credit is never explained. One says consumption jumped after a model change without notice, then cancelled, citing Copilot's flat $39/month plan as better value — [JetBrains support forum](https://intellij-support.jetbrains.com/hc/en-us/community/posts/31142845556370)
  - Thread "AI Assistant consuming credits much faster since WebStorm 2025.2.1" includes a report of the same file being sent about 30 times in one prompt. A JetBrains staffer says spikes may be bugs and mentions planned BYOK (bring your own key) — [JetBrains support forum](https://intellij-support.jetbrains.com/hc/en-us/community/posts/29142783285266?page=2)
  - One Ultimate subscriber waited more than 2 weeks for a quota extension and called the system broken — [JetBrains support forum](https://intellij-support.jetbrains.com/hc/zh-cn/community/posts/26179850596882/comments/27619602281106)
- Plugin breakage: from 2025.3, Community (IC) build artifacts are gone, so plugin developers must retarget the unified artifact — [JetBrains Platform blog](https://blog.jetbrains.com/platform/2025/11/intellij-platform-2025-3-what-plugin-developers-should-know/); [JetBrains help](https://www.jetbrains.com/help/idea/2025.3/intellij-idea-single-distribution.html)

### Inferences
- Users value predictability. Opportunities for a lightweight editor:
  - never block editing or navigation on indexing; keep basic features available instantly;
  - incremental, non-blocking reindex on branch switch;
  - stable UI with an option to keep layouts;
  - transparent or BYOK AI pricing.
- The New UI fight shows that forcing a layout change on an installed base is costly, even when most users adopt it. Ship UI changes as opt-in with a long tail of legacy support.
- Cost complaints and Copilot comparisons suggest bundling AI into a paid IDE with opaque metering backfires. BYOK or a flat price is preferred.

### Gaps
- No quantitative 2025-2026 measurements of IntelliJ startup time or RAM versus VS Code were found.
- No HN thread on IntelliJ performance in 2025 was located.
- No data was found on plugin breakage rates across major versions.

## RustRover: reception since GA, free non-commercial license, vs rust-analyzer, Rust-specific features and complaints

### Takeaway
RustRover went GA May 21, 2024 with a free non-commercial license. Reception is split:
- Fans praise an integrated GUI experience: macro expansion, Cargo menus, test runner, extra refactors like "move function to another file".
- Critics dislike the closed-source, Kotlin-based analyzer (not rust-analyzer) and the debugger weaknesses (LLDB, especially on Windows), and many stay with free, editor-agnostic rust-analyzer.
- 2025-2026 releases focused on the debugger, analysis false positives, and AI.

### Cited Findings
- GA May 21, 2024. Free for individual non-commercial use under an honor system. Commercial use, including OSS work done as part of a paid job, needs a paid license. Non-commercial users cannot opt out of anonymous usage statistics — [JetBrains blog](https://blog.jetbrains.com/rust/2024/05/21/rustrover-is-released-and-includes-a-free-non-commercial-option/); [InfoQ](https://www.infoq.com/news/2024/06/rust-jetbrains-ide)
- Critics noted web plugin support and tooling were removed, with no way to install them even for paying users — [feddit/lemmy thread](https://feddit.it/post/8933525)
- RustRover uses JetBrains' own analyzer written in Kotlin, not rust-analyzer. The formerly open-source IntelliJ Rust plugin became closed when folded into RustRover, which some saw as "further distancing themselves from Rust-Analyzer" — [Lobsters](https://lobste.rs/s/avzof2/rustrover_is_released_includes_free_non); [users.rust-lang.org](https://users.rust-lang.org/t/jetbrains-rust-plugin-rust-rover/99843)
- An earlier user switched from IntelliJ Rust to VS Code + rust-analyzer because it "was performing better" — [users.rust-lang.org](https://users.rust-lang.org/t/jetbrains-rust-plugin-rust-rover/99843)
- "Rust Rover users: how do you rate it?" thread (Oct 2024), anecdotal:
  - Likes:
    - extra and more flexible refactors (move function to another file);
    - "much easier to use than VS Code";
    - visual code awareness, including macro expansion and type detection;
    - Cargo commands in menus;
    - integrated unit tests;
    - all settings in the GUI;
    - one user calls the analysis "a tie" with rust-analyzer.
  - Dislikes:
    - Extract Function sometimes yields non-compiling code (async signatures);
    - "Inspect code" confused by newly stabilized syntax;
    - fewer language plugins than VS Code;
    - different keybindings;
    - debugger: variables sometimes unavailable, `Option<T>` hard to read on Windows, breakpoints sometimes failing (LLDB adaptation), with VS Code "sometimes better";
    - many cosmetic bugs, sometimes fixed by clearing caches.
  - A VS Code user found rust-analyzer good but debugging hard, so relies on println!.
  - Source: [users.rust-lang.org](https://users.rust-lang.org/t/rust-rover-users-how-do-you-rate-it/119074)
- Critics of the paid tool prefer neovim + rust-analyzer + ra-multiplex, and note rust-analyzer works in any LSP editor — [feddit thread](https://feddit.it/post/8933525); [users.rust-lang.org](https://users.rust-lang.org/t/rust-rover-users-how-do-you-rate-it/119074)
- Release cadence (secondhand mirror of JetBrains notes):
  - 2025.2 (Aug 5, 2025): Junie agent 30% faster with MCP; remote debugging via GDB/LLDB/OpenOCD; Rename Cargo Package.
  - 2025.3 (Dec 9, 2025): new LLDB expression-evaluation engine for a wider range of Rust expressions.
  - 2026.1 (Mar 30, 2026): fewer false positives in code analysis; cargo-nextest support.
  - 2026.1.3 (Jun 9, 2026): "Don't step into libs" option; const impl support.
  - Source: [ComponentSource RustRover releases](https://www.componentsource.com/product/rustrover/releases)

### Inferences
- JetBrains' own Rust roadmap concentrated on the debugger and on reducing false positives for two years. Those are the weak spots, and both are where rust-analyzer + VS Code is also weak, especially debugging.
- A Rust-native editor could stand out with:
  - polished, built-in debugging (CodeLLDB/DAP with good Rust pretty-printers for `Option`/`Vec`/enums, "don't step into std");
  - one-click Cargo/test/nextest integration;
  - macro-expansion views.
  rust-analyzer already provides expand-macro, so the work is UX.
- The rust-analyzer camp values openness and editor-agnosticism. A new editor built on rust-analyzer avoids the "closed analyzer" criticism and inherits upstream fixes.

### Gaps
- No quantitative RustRover adoption figures were found (e.g., Rust survey IDE share).
- No 2025-2026 head-to-head benchmark of RustRover vs rust-analyzer (memory, latency) was found.
- No primary JetBrains blog text for 2025-2026 RustRover releases; data comes from the ComponentSource mirror.
- Status of releases after June 2026 (e.g., 2026.2) is unverified.

## Fleet: promise, reception, discontinuation, lessons

### Takeaway
Fleet is discontinued. It was announced Dec 8, 2025, and downloads ended Dec 22, 2025.
- It launched in preview in November 2021 as a lightweight, distributed, collaborative, multi-language IDE meant to answer VS Code.
- It was repositioned twice: first as "editor with smart assistance", then as an AI-first editor.
- It never left preview.
- JetBrains concluded it could neither replace IntelliJ nor find a differentiated niche against VS Code and its AI forks.
- Its platform now underpins JetBrains Air, an agentic tool.

### Cited Findings
- JetBrains "could not justify maintaining two general-purpose IDEs". It could "neither replace IntelliJ with Fleet nor define a differentiated niche for the Fleet IDE". Two overlapping lines "created confusion and diluted focus" — [InfoWorld](https://www.infoworld.com/article/4104991/jetbrains-discontinues-fleet-ide.html)
- From user research, JetBrains judged another AI editor would struggle to stand out "particularly in a market filled with AI-first Visual Studio Code forks". JetBrains-IDE users needed a much stronger reason to switch — [InfoWorld](https://www.infoworld.com/article/4104991/jetbrains-discontinues-fleet-ide.html)
- Existing installs keep running, but server-dependent features such as AI Assistant may stop working over time. Fleet components and UX ideas were reused in other JetBrains IDEs — [InfoWorld](https://www.infoworld.com/article/4104991/jetbrains-discontinues-fleet-ide.html); [AlternativeTo](https://alternativeto.net/news/2025/12/jetbrains-will-discontinue-its-fleet-ide-on-december-22-2025-due-to-overlapping-purposes/)
- Fleet stayed in public preview for years. A Fleet-based Kotlin Multiplatform IDE was dropped in Feb 2025 in favor of IntelliJ.
  - Adoption obstacles cited: the IntelliJ plugin ecosystem keeping users loyal, and the long preview.
  - Some fans called the shutdown "a wrong move" and said they were leaving JetBrains.
  - Others wanted agentic features in existing IDEs instead of new products.
  - Source: [DevClass](https://devclass.com/2025/12/09/jetbrains-abandons-fleet-ide-pins-hopes-on-forthcoming-air-agentic-development-tool/)
- JetBrains Air was in public preview at the Dec 2025 announcement. It is built on the Fleet platform and is an agent-supervision workflow requiring an Anthropic subscription. It was macOS-only then, with Windows/Linux, other agents (Codex, Gemini, Junie), and a cloud mode planned — [DevClass](https://devclass.com/2025/12/09/jetbrains-abandons-fleet-ide-pins-hopes-on-forthcoming-air-agentic-development-tool/)
- Wikipedia lists Fleet as officially discontinued in 2025 — [Wikipedia](https://en.wikipedia.org/wiki/JetBrains)

### Inferences
Lessons for a new VS Code-style editor:
1. Being "lighter, modern architecture" is not enough. Even JetBrains, with brand and tech, could not pull users from VS Code.
2. Extension ecosystem and switching costs are the moat. Compatibility with VS Code extensions, keymaps, settings, themes, or LSP/DAP lowers switching cost.
3. Repositioning repeatedly (lightweight, then smart editor, then AI-first) and staying in perpetual preview erode trust.
4. A clear niche is required. For a Rust editor, that could be raw performance, low RAM, and first-class Rust/Cargo tooling.
5. Avoid server-side dependencies that die with the product.

### Gaps
- No quantitative Fleet usage numbers were found.
- Detailed user complaints about Fleet's feature gaps or performance were not retrieved.
- The Air status as of Oct 2026 is unverified; there was only an InfoWorld headline dated Sep 23, 2026, without content.

## Visual Studio (full IDE): debugger praise, Windows-only, heaviness

### Takeaway
Visual Studio is the #2 IDE in the SO 2025 survey at 29%. It is regarded as having the most mature debugger, especially for Windows/.NET/WPF, but is known as heavy. Visual Studio 2026 (GA Nov 11, 2025) targeted performance: Microsoft claims UI hangs were cut by more than half, and early users praised the speed. Rider is the cross-platform alternative, with mixed speed reports.

### Cited Findings
- Visual Studio is used by 29% of SO 2025 respondents — [Stack Overflow Survey 2025](https://survey.stackoverflow.co/2025/)
- Visual Studio 2026 went GA Nov 11, 2025. Microsoft's claims:
  - UI hangs reduced by more than 50% vs VS 2022;
  - more than 5,000 bugs fixed and 300 feature requests added;
  - faster startup, project open, navigation, and builds on x64 and Arm64.
  - Source: [Visual Studio Magazine](https://visualstudiomagazine.com/articles/2025/11/12/visual-studio-2026-ga-first-intelligent-developer-environment-ide.aspx); [InfoWorld](https://www.infoworld.com/article/4088876/microsoft-releases-ai-native-visual-studio-2026.html)
- An early previewer said "HOLY […] this is FAST". Reviewers note users still want more memory/performance work and that parts still depend on the older .NET Framework. A 40% large-solution load improvement is cited by a secondary blog only — [smallbiztrends](https://smallbiztrends.com/visual-studio-2026-insiders-unveils-ai-integration-and-speed-boosts/); [TrustedTech](https://trustedtechteam.com/blogs/news/visual-studio-2026-everything-you-need-to-know)
- Rider vs Visual Studio:
  - A (vendor-adjacent, ~2021) comparison says Rider starts and compiles faster, and VS freezes on solutions with many projects — [codingsight](https://codingsight.com/visual-studio-vs-jetbrains-rider-a-detailed-comparison/)
  - A 2025 user found Rider's debugger slow to start for an Avalonia app while VS launched almost immediately. JetBrains replied Rider has its own debugger implementation, with performance fixes in 2025.3.2 — [Rider support](https://rider-support.jetbrains.com/hc/en-us/community/posts/31170113981714/comments/33078213141650)
  - A user migrating from VS for Mac found Rider much slower on a 6-project solution and on debug startup — [Rider support](https://rider-support.jetbrains.com/hc/en-us/community/posts/18394635703058/comments/18418447115282)

### Inferences
- The VS lesson is that the debugger is the IDE's crown jewel. Good debug UX (fast launch, rich visualizers, conditional/log breakpoints, exception settings) drives loyalty even when the rest is heavy.
- Microsoft's own pivot to "fewer hangs, faster startup" in VS 2026 confirms that responsiveness is a top complaint even among loyal heavyweight-IDE users.

### Gaps
- No independent 2025-2026 benchmark of VS vs VS Code vs Rider debugger or RAM was found.
- No primary source for Windows-only complaints was found. Visual Studio for Mac's retirement is relevant context but was not sourced here.
- No quantitative debugger-satisfaction survey was found.
