# Code Editor Surveys and Trends: Usage, Satisfaction, AI, Privacy, Performance, Rust (data as of Oct 2026)

## Stack Overflow Developer Survey 2024, 2025, 2026: IDE usage and admired/desired

### Takeaway
VS Code has led on usage every year, with 73.6% in 2024, 75.9% in 2025 and 70.9% in 2026. In 2026 its admired score (55.7%) is only mid-pack. Neovim (69.9%), Zed (57.3%) and Rider (56.9%) are the most admired. AI-first forks score poorly on admiration: Cursor 33.0%, Windsurf 22.7%, Kiro 31.2%. Zed is the fastest-growing editor, rising from 7.3% in 2025 to 11.4% in 2026.

### Cited Findings
**2026 (published ~Oct 2026; IDE question n = 13,193; whole survey >30,000 respondents from 169 countries).** Columns: used / desired / admired.
- VS Code: 70.9% / 40.6% / 55.7% — [SO 2026 Technology data](https://survey.stackoverflow.co/2026/technology/data)
- Notepad++: 27.8% / 15.1% / 52.5% — same source
- Visual Studio: 27.6% / 14.1% / 47.8% — same source
- Vim: 27.2% / 16.4% / 55.7% — same source
- IntelliJ IDEA: 24.3% / 14.5% / 53.3% — same source
- Cursor: 14.9% / 7.9% / 33.0% — same source
- Neovim: 14.7% / 12.9% / 69.9% (highest admired in the table) — same source
- Zed: 11.4% / 9.7% / 57.3% — same source
- Sublime Text: 9.8% / 5.4% / 49.6% — same source
- Rider: 7.0% / 5.2% / 56.9% — same source
- Windsurf: 3.2% / 1.7% / 22.7% — same source
- Kiro: 2.8% / 1.7% / 31.2% — same source
- Admired is defined as "Have used" & "Want to use". — same source
- 76.5% of respondents (n = 16,241) said they "used a development environment (IDE) to write code or interact with agent-assisted developer tools". — [SO 2026 Technology](https://survey.stackoverflow.co/2026/technology)
- Trade-press coverage reports Claude Code at 65.5% among AI coding agents, then Copilot, OpenAI Codex (29.5%) and Cursor (25.5%). This comes from a search snippet; the article itself returned 403 and the figures were not verified on the primary page. — [Visual Studio Magazine, 6 Oct 2026](https://visualstudiomagazine.com/articles/2026/10/06/stack-overflow-2026-survey-vs-code-leads-visual-studio-takes-third.aspx)
- The headline says Visual Studio "takes third", but the raw counts give Notepad++ (27.8%) a slight lead over Visual Studio (27.6%). — [SO 2026 data](https://survey.stackoverflow.co/2026/technology/data)

**2025 (~49,000 respondents).** Columns: all respondents / professional developers / professionals who use AI.
- VS Code: 75.9% / 76.2% / 77.7% — [SO 2025 Technology](https://survey.stackoverflow.co/2025/technology)
- Visual Studio: 29% / 29.7% / 29% — same source
- IntelliJ IDEA: 27.1% / 28.4% / 29.1% — same source
- Notepad++: 27.4% / 26.9% / 26% — same source
- Vim: 24.3% / 24% / 23.3% — same source
- Cursor: 17.9% / 19.3% / 21.7% — same source
- Neovim: 14% / 13.6% / 13.5% — same source
- Sublime Text: 10.5% / 10.5% / 10.8% — same source
- Claude Code: 9.7% / 10% / 11.3% — same source
- Zed: 7.3% / 7.5% / 7.9% — same source
- Windsurf: 4.9% / 5% / 5.7% — same source

**2024.** Columns: all respondents / professional developers.
- VS Code 73.6% / 74%; Visual Studio 29.3% / 29.7%; IntelliJ 26.8% / 27.9%; Notepad++ 23.9% / 24%; Vim 21.6%; Android Studio 16.1%; PyCharm 15.1%; Jupyter 12.8%; Neovim 12.5% / 11.4%; Sublime 10.9%; Eclipse 9.4%; Xcode 9.3%; Nano 9.2%; WebStorm 7.5%; Rider 5.7%; VSCodium 4.8%; CLion 4.6%; Emacs 4.2%; Fleet 1.7%; Helix 1.7% / 1.5%. — [SO 2024 Technology](https://survey.stackoverflow.co/2024/technology)

### Inferences
- VS Code usage fell about 5pp from 2025 to 2026, while Zed rose about 4pp. Part of this may be sample change, since the 2026 IDE question had only 13k respondents versus about 49k overall in 2025. Treat the year-over-year comparison with caution.
- Cursor fell from 17.9% (2025) to 14.9% (2026), and its admired score is low (33%). Its users try it but do not love it. "AI-first fork of VS Code" does not on its own produce loyalty.
- The highest-admired tools (Neovim, Zed, Rider, Vim) share speed, keyboard-centric efficiency and a strong identity. This supports a performance-first Rust editor.
- VSCodium's 4.8% in 2024 shows a real niche that wants VS Code without Microsoft telemetry and branding. It is a proxy for demand for privacy and open source.

### Gaps
- 2025 and 2024 IDE admired/desired percentages could not be retrieved: the page was truncated and the /data URL returned 404.
- 2026 professional-only IDE splits were not retrieved.
- Claude Code is absent from the 2026 IDE table.

## AI tool usage and trust/sentiment (Stack Overflow, JetBrains)

### Takeaway
AI use is near-universal and still rising. Trust is low and falling. Developers trust AI output mainly when they can easily check it, which favors review-friendly features (diffs, attribution, easy undo) over autonomous magic.

### Cited Findings
- 2025: 84% use or plan to use AI tools, up from 76% in 2024. — [SO 2025 AI](https://survey.stackoverflow.co/2025/ai)
- 2025: 51% of professional developers use AI daily (47.1% of all respondents). — same source
- 2025: favorable sentiment is 60%, down from more than 70% in 2023 and 2024. — same source
- 2025: 33% trust AI accuracy (3.1% "highly trust") and 46% distrust it (19.6% "highly distrust"). Experienced developers have the lowest high-trust rate at 2.6%. — same source
- 2024 trust was 43%, according to secondary coverage. — [tech.co](https://tech.co/news/survey-developers-using-ai-more-trust-less); [LeadDev](https://leaddev.com/technical-direction/trust-in-ai-coding-tools-is-plummeting)
- 2025 frustrations: 66% cite "AI solutions that are almost right, but not quite", and 45.2% say debugging AI-generated code is more time-consuming. — [SO 2025 AI](https://survey.stackoverflow.co/2025/ai)
- 2025 agents: about 31% use agents at work (14.1% daily), and 37.9% do not plan to use agents. — same source
- 2026 trust:
  - only 6.6% would trust AI for important decisions
  - nearly half trust output they can easily verify
  - 78.7% say source attribution is important or very important

  Source: [IT Brief UK, 6 Oct 2026](https://itbrief.co.uk/story/stack-overflow-survey-shows-developers-trust-ai-with-caveats). InfoWorld rounds these to 7% and 48%: [InfoWorld](https://www.infoworld.com/article/4233357/ai-distrust-is-githubs-opportunity.html).
- 2026 usage:
  - 66% use coding assistants or agents
  - 63% use general chat tools
  - 26.2% use automated agent workflows
  - only 17.2% use no AI tools
  - 31% spend 4 or more hours a day with AI tools

  Source: [IT Brief UK](https://itbrief.co.uk/story/stack-overflow-survey-shows-developers-trust-ai-with-caveats)
- 2026 tasks and policy:
  - implementation tasks rated useful by 74%
  - debugging/refactoring rated useful by 66%
  - only 24% of employers have published approved tools or formal AI guidance
  - among developers who switched tools, 25% cited output quality and 11% an organisational requirement
  - AI features were cited by only 2% as a reason to recommend a technology

  Source: same as above.
- 2026 model providers: 75% currently use Anthropic models and 74% OpenAI. 49.0% want to work with Anthropic next year, versus 32% for OpenAI. — same source
- JetBrains Developer Ecosystem 2025 (24,534 developers, 194 countries):
  - 85% use AI regularly
  - 62% rely on at least one AI coding assistant, agent or AI editor
  - 15% have not adopted AI tools

  Sources: [JetBrains Research blog](https://blog.jetbrains.com/research/2025/10/state-of-developer-ecosystem-2025/); [InfoWorld via scour](https://scour.ing/p/https://www.infoworld.com/article/4077352/85-of-developers-use-ai-regularly-jetbrains-survey.html)
- JetBrains 2025 top AI concerns:
  - code quality 23%
  - limited understanding of complex logic 18%
  - privacy and security 13%
  - negative effect on coding skills 11%
  - lack of context awareness 10%
  - job security 9%

  Source: [i-programmer](https://www.i-programmer.info/news/99-professional/18422-what-does-jetbrains-survey-tell-us-about-ai.html); [abp.io summary](https://abp.io/community/articles/jetbrains-state-of-developer-ecosystem-report-2025-key-z0638q5e)

### Inferences
- AI integration is table stakes for adoption, since about 83% use AI in 2026. But "AI-ness" barely drives recommendation (2%), and AI-first editors have low admiration. The roadmap should be an excellent core editor plus pluggable, verifiable AI, not an AI-first product.
- High attribution demand (78.7%) and verify-first trust point to useful features: inline diff review, provenance labels, per-hunk accept/reject and agent action logs.
- Only 24% of employers have AI policies, and 11% of tool switches are mandated by organisations. Enterprise controls matter: an admin-enforceable AI off switch, an allow-list of providers and bring-your-own-key (BYOK).
- Multi-provider support is required: Anthropic and OpenAI are near parity on usage.

### Gaps
- The primary SO 2026 AI page was not fetched; 2026 AI figures come from secondary press.
- No 2026 JetBrains Developer Ecosystem figures were found.

## Developer attitudes to AI in editors: opt-out, privacy, local models

### Takeaway
No systematic survey quantifies demand for a full AI off switch or for local models. Vendor actions and forum demand show it is real, and Zed shipped a global `disable_ai` setting in July 2025 in response.

### Cited Findings
- On 23 July 2025, Zed added a global `"disable_ai": true` setting and planned an onboarding toggle. — [Zed blog](https://zed.dev/blog/disable-ai-features)
- Zed's stated reasons from users:
  - concerns about training data, environmental impact and philosophical objections to machine-generated code
  - preference for predictable tools
  - organisational or legal bans on AI for proprietary code
  - approved vendors Zed does not support

  Source: [Zed blog](https://zed.dev/blog/disable-ai-features); [GitHub discussion #16750](https://github.com/zed-industries/zed/discussions/16750)
- Privacy and security is the #3 AI concern at 13% (JetBrains 2025). — [i-programmer](https://www.i-programmer.info/news/99-professional/18422-what-does-jetbrains-survey-tell-us-about-ai.html)
- 37.9% of developers do not plan to use AI agents (SO 2025). — [SO 2025 AI](https://survey.stackoverflow.co/2025/ai)
- Non-developer products show user backlash when AI is enabled by default with hidden disable options (Firefox support thread). — [Mozilla support](https://support.mozilla.org/sr/questions/1555695)

### Inferences
- A sizeable minority wants AI fully absent or controllable: 17.2% use no AI (2026) and 37.9% reject agents (2025). Ship AI as opt-in or easily globally disabled, with BYOK and local-model endpoints such as Ollama or OpenAI-compatible servers.

### Gaps
- No quantitative survey found on local-model preference or on demand for a "disable all AI" option.

## Telemetry/privacy and open-source licensing preferences

### Takeaway
Hard survey data is sparse. The best quantitative proxy is VSCodium usage: 4.8% in 2024, a VS Code rebuild without Microsoft telemetry and branding.

### Cited Findings
- VSCodium was used by 4.8% of all respondents and 3.9% of professionals in 2024. — [SO 2024 Technology](https://survey.stackoverflow.co/2024/technology)
- Privacy and security is 13% of top AI concerns (JetBrains 2025). — [i-programmer](https://www.i-programmer.info/news/99-professional/18422-what-does-jetbrains-survey-tell-us-about-ai.html)

### Inferences
- Default-off or transparent telemetry and an OSI license are differentiators for a niche that is vocal but measurable at about 5%. They also unlock Linux distro packaging.

### Gaps
- No survey found that directly measures attitudes to editor telemetry or license preference (MIT/Apache vs GPL vs source-available).

## Rust developers' editor preferences and pain points

### Takeaway
Exact Rust editor shares were not retrievable: the charts are images and the JetBrains pages returned 403. The direction is clear. VS Code with rust-analyzer still leads, but it is losing share (down 5pp in 2024). Zed made a "remarkable jump" in 2025, with Helix a "good second", and agentic editors are eroding VS Code and IntelliJ. Top pain points are slow compiles, disk usage, debugging, and rust-analyzer/rustfmt resource use.

### Cited Findings
- 2025 State of Rust (7,156 responses, 17 Nov to 17 Dec 2025, published 2 Mar 2026):
  - Zed "did a remarkable jump upward"
  - Helix is a "good second"
  - agentic editors are "eroding the userbase of VSCode and IntelliJ"

  Source: [Rust blog 2025 results](https://blog.rust-lang.org/2026/03/02/2025-State-Of-Rust-Survey-results); [iThome](https://www.ithome.com.tw/news/174178)
- 2025 State of Rust problems: slow compile times and storage use remain the top problem. Debugging fell from 2nd to 4th (about 2pp), and a dedicated debugging survey was started. — [Rust blog 2025](https://blog.rust-lang.org/2026/03/02/2025-State-Of-Rust-Survey-results)
- 2025 usage: 91.7% use Rust (92.5% in 2024), and 55.1% use it daily or nearly daily (53.4% in 2024). — [InfoWorld](https://www.infoworld.com/article/4139528/rust-developers-have-three-big-worries-survey.html); [Rust blog](https://blog.rust-lang.org/2026/03/02/2025-State-Of-Rust-Survey-results)
- 2024 State of Rust: VS Code "still remains the most popular option, its share has dropped by 5pp", and Zed "gained considerable traction". — [Rust blog 2024 results](https://blog.rust-lang.org/2025/02/13/2024-State-Of-Rust-Survey-results/)
- 2024 State of Rust problems: slow compilation is the top limiting factor, followed by "subpar support for debugging Rust" and high disk usage. Open answers mention "Rust tooling being slow or resource intensive (rust-analyzer and rustfmt)" and a wish for "a better IDE story". — same source
- JetBrains Rust Ecosystem 2025 (published Feb 2026): 89% have tried at least one AI tool, and 78% actively use AI coding assistants. The IDE shares in this report could not be retrieved (403). — [JetBrains blog (search snippet)](https://blog.jetbrains.com/rust/2026/02/11/state-of-rust-2025/)
- RustRover uses JetBrains' own Rust analysis engine, not rust-analyzer. It is free for non-commercial use and paid for commercial use. — [JetBrains, RustRover vs VS Code, Oct 2026 (vendor source)](https://blog.jetbrains.com/rust/2026/10/01/rustrover-vs-vs-code/)
- Helix was used by 1.7% of all developers in 2024 (SO). — [SO 2024](https://survey.stackoverflow.co/2024/technology)

### Inferences
- Rust developers already move toward fast native editors (Zed, Helix), which is the target market. They need first-class rust-analyzer integration that stays responsive while rust-analyzer is busy. A built-in debugger (CodeLLDB/DAP) is a known gap, as are cargo-aware UI and low memory use.

### Gaps
- Exact Rust-survey editor percentages (VS Code, RustRover, Zed, Neovim, Helix) are in chart images or PDFs and JetBrains pages that were not accessible. Get them from the full report PDFs.

## Accessibility

### Takeaway
No quantitative ratings of editor accessibility were found. Qualitative research says screen readers can use VS Code and Visual Studio, but much information (live errors, refactor hints) stays visual.

### Cited Findings
- Microsoft Research CodeTalk: IDEs including VS Code are screen-reader accessible, but real-time error alerts and refactoring suggestions are not announced. — [Microsoft Research](https://www.microsoft.com/en-us/research/blog/codetalk-rethinking-ide-accessibility)
- Microsoft devblog cites WebAIM: JAWS is the most-used screen reader; key themes are screen reader compatibility, contrast and keyboard-shortcut documentation. — [devblogs](https://devblogs.microsoft.com/visualstudio/wp-json/wp/v2/posts/633)
- Community survey topics for blind coders: indentation, tab handling, long-line navigation. — [CodeMirror forum](https://discuss.codemirror.net/t/code-editor-screen-reader-accessiblity-survey/1790)

### Inferences
- GPU-rendered Rust editors (Zed-style) must build an accessibility tree, for example via AccessKit. Otherwise they lose screen-reader users that VS Code (DOM-based) serves well. Audio cues and accessible diff/diagnostic views are the bar.

### Gaps
- No WebAIM developer-specific stats were found, and no comparative accessibility scores across editors.

## Most-wanted features, performance, and reasons for switching

### Takeaway
No current survey ranks editor features. Proxies point to performance and responsiveness as the strongest admiration driver. 2026 data shows tool switches driven by AI output quality (25%) and organisational mandates (11%).

### Cited Findings
- Latency study (Pavel Fatin, 2015):
  - Human visual processing takes about 40 ms, and even 20 ms of added latency matters.
  - Typical keyboard and monitor I/O alone is about 26 ms.
  - Aim for 1 to 5 ms of editor painting.

  Source: [Typing with pleasure](https://pavelfatin.com/typing-with-pleasure/)
- Measured average typing latency on Windows (text file), from the same 2015 study:
  - GVim 0.9 ms
  - IntelliJ zero-latency mode 2.9 ms (default mode 24.7 ms, max 83.7 ms)
  - Notepad++ 4.3 ms
  - Sublime 8.2 ms
  - Atom (Electron) 49.4 ms

  Source: [Typing with pleasure](https://pavelfatin.com/typing-with-pleasure/)
- 2026: of developers who changed tools, 25% cited output quality and 11% an organisational requirement. — [IT Brief UK](https://itbrief.co.uk/story/stack-overflow-survey-shows-developers-trust-ai-with-caveats)

### Inferences
- The Atom (Electron) versus native gap of about 10x in 2015 data supports a native Rust renderer as a core value proposition. Measured keystroke-to-photon latency should be a published benchmark.
- The admired rankings (Neovim 69.9%, Zed 57.3%) indicate that speed and keyboard efficiency matter more than feature count.

### Gaps
- No 2024 to 2026 survey ranking editor features (debugger, git, remote dev, multi-cursor) was found.
- No modern (2024 to 2026) independent editor latency benchmark covering VS Code, Zed and Cursor was found.
- No survey data on reasons for switching editors specifically, beyond the 2026 AI tool-switch figures.
