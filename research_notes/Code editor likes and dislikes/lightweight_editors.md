# Lightweight and General-Purpose Editors: Developer Sentiment (Notepad++, Kate, Geany, Windows Notepad, gedit/GNOME Text Editor, BBEdit, TextMate, Micro, Cloud Editors)

Research date: October 2026. Search tooling returned mostly press, vendor docs, review aggregators and forum posts; direct Reddit/HN threads were rarely surfaced (noted under Gaps).

## Notepad++: why it stays popular, top plugins, complaints

### Takeaway
Notepad++ is still the #2-3 most-used editor among developers (about 27-28% in Stack Overflow surveys 2025-2026), loved for instant startup, being free, tabbed editing, regex find/replace and a plugin ecosystem. Main negatives: Windows-only, dated UI, features gated behind plugins, a 2025 hijacked-updater supply-chain incident, and the author's political release messaging.

### Cited Findings
- Stack Overflow 2025 survey: Notepad++ about 27.4% usage, 3rd among IDEs/editors; VS Code 75.9%, Visual Studio 29%. Multi-select question, so totals exceed 100%. Sample size reported inconsistently (26,143 vs 49,000+). — [Visual Studio Magazine, Aug 2025](https://visualstudiomagazine.com/articles/2025/08/01/stack-overflow-dev-survey-visual-studio-vs-code-hold-of-ai-ides-to-remain-on-top.aspx)
- Stack Overflow 2026 survey (released 6 Oct 2026): Notepad++ 27.8%, narrowly ahead of Visual Studio at 27.6%; VS Code 70.9% (13,193 responses to the editor question). Figures come from a search summary; the article page returned HTTP 403 when fetched, so they are not independently verified. — [Visual Studio Magazine, Oct 2026](https://visualstudiomagazine.com/articles/2026/10/06/stack-overflow-2026-survey-vs-code-leads-visual-studio-takes-third.aspx)
- Review themes: "opens immediately", lightweight, speed, completely free, regex find/replace, tabbed multi-file editing, "notepad on steroids". Most common complaints: dated interface, advanced features need plugins, and VS Code is more productive for serious coding. — [Capterra reviews](https://www.capterra.com/p/185965/Notepad/reviews/); [G2 review](https://www.g2.com/products/notepad/reviews/notepad-review-12469435)
- HN anecdote (2013, old): a user stayed on Notepad++ over Sublime Text 3 because of its native feel and because it is "a lot faster, especially in startup". — [Hacker News](https://news.ycombinator.com/item?id=5961901)
- Plugins: a community moderator recommends XML Tools for XML formatting and JsonTools for JSON. Others call NppExec essential: it pipes the active file through a compiler or linter and shows the output. PythonScript and LuaScript are recommended for scripting. — [Notepad++ Community](https://community.notepad-plus-plus.org/post/104851); [Notepad++ Community](https://community.notepad-plus-plus.org/post/54215)
- No reliable plugin download counts were found. One third-party guide labels XML Tools and JSON Viewer "Very High" downloads but gives no method. — [glama.ai guide (weak)](https://glama.ai/mcp/servers/@sandraschi/tailscale-mcp/blob/d5641c95adb6db5d857de6be59b2dd23605a1005/docs/notepadpp/PLUGIN_ECOSYSTEM_COMPREHENSIVE.md)
- Security incident (disclosed Feb 2026): attackers hijacked update delivery for selected users from about June to early December 2025 through a compromised shared-hosting provider. Root causes were the hosting compromise plus weak updater integrity checks. Source code and build were not compromised. Users who updated manually were safe. — [The Register](https://www.theregister.com/2026/02/02/notepad_plusplus_intrusion/); [BleepingComputer](https://www.bleepingcomputer.com/news/security/notepad-plus-plus-update-feature-hijacked-by-chinese-state-hackers-for-months/); [The Hacker News](https://thehackernews.com/2026/02/notepad-official-update-mechanism.html)
- Attribution conflicts: Rapid7 says Lotus Blossom; The Hacker News reports a link to Violet Typhoon (APT31). Remediation: new host, rotated credentials, hardened updater, GlobalSign-only signing, upgrade to 8.9.1+ advised. — [Pureinfotech](https://pureinfotech.com/notepad-plus-plus-compromised-fix/); [PCWorld](https://www.pcworld.com/article/3049744/hijacked-notepad-updater-quietly-targeted-users-for-months.html)
- Political messaging: the 2019 "Free Uyghur" edition led to spam floods that crashed the site for hours. Earlier political editions included a 2008 Beijing Olympics boycott message and a 2014 Tiananmen edition. — [Taiwan News](https://www.taiwannews.com.tw/news/3808148)
- Windows-only gap: in May 2026 an unauthorized "Notepad++ for Mac" port was disavowed by Don Ho and forced to rebrand (reported as "NextPad++"). The episode shows demand for Notepad++ on macOS. — [Neowin](https://www.neowin.net/news/notepad-creator-issues-warning-to-users-about-an-unauthorized-macos-fork/); [Slashdot/Ars](https://tech.slashdot.org/story/26/05/04/232252/notepad-for-mac-release-is-disavowed-by-the-creator-of-the-original); [heise](https://heise.de/-11280446)
- Linux reimplementation Notepad Next (Qt; Flatpak/AppImage) exists, but its developer advises against using it for critical work, and parity is incomplete. Notepadqq is another clone. — [It's FOSS](https://itsfoss.com/notepad-next); [UbuntuHandbook](https://ubuntuhandbook.org/index.php/2022/04/notepad-plus-plus-linux-support/); [DebugPoint](https://www.debugpoint.com/notepad-replacement-ubuntu/)

### Inferences
- Roughly 1 in 4 developers still keeps Notepad++ alongside a main IDE. That suggests a lasting "scratchpad / quick-open / log viewer" role that heavy IDEs do not fill. A fast Rust editor that starts as fast as Notepad++ and runs cross-platform could take this role on macOS and Linux, where people keep asking for Notepad++ clones.
- The updater incident is a design lesson: signed updates with strict verification (signature pinning) are a must-have, not an extra.
- Plugin demand clusters around format/validate (JSON, XML), run external tool (NppExec), compare/diff, and scripting. These are good candidates to build in.

### Gaps
- No quantitative plugin popularity data (Plugin Admin has no public counts). The Compare plugin's popularity is widely assumed but was not sourced here.
- Reddit r/notepadplusplus sentiment was not surfaced by search.

## Kate (and KWrite): praised features and complaints

### Takeaway
Kate is praised as a full-featured but light editor: built-in LSP client, per-session saved splits and layouts, multiple named sessions, and good big-file handling. Complaints found mostly concern per-language LSP configuration and formatting glitches.

### Cited Findings
- KDE developer Akseli Lahtinen (Apr 2025) uses Kate "for pretty much all the programming projects I do". He describes his LSP client setup and notes that splits are saved per session, so each session keeps its own layout. — [Planet KDE](https://planet.kde.org/akseli-lahtinen-aks-2025-04-20-how-i-use-kate-editor)
- A Linux Mint user review says Kate handles big files "without hiccups", manages sessions, and has strong integrated features. — [Linux Mint Community](https://community.linuxmint.com/software/view/org.kde.kate)
- KDE Discuss threads from 2025 include: Puppet language server not working, LSP YAML configuration issues, and Kate jumping to end of file when pylsp formats on save. Only thread titles were seen. — [KDE Discuss](https://discuss.kde.org/tag/kate)
- Historic note: early LSP in Kate 19.12 had configuration problems, fixed in point releases. — [Phoronix 2020](https://www.phoronix.com/news/Kate-LSP-2020)
- Kate is positioned (in Spanish press) as KDE's answer to VS Code as a development tool. — [Genbeta](https://www.genbeta.com/desarrollo/kde-apuesta-nueva-version-su-editor-texto-kate-para-plantar-cara-a-visual-studio-code-como-herramienta-desarrollo)

### Inferences
- Kate shows that "lightweight editor + LSP + sessions + splits" is a viable middle tier. LSP config friction (JSON/YAML server settings) is its main pain point. Zero-config LSP discovery would beat it.

### Gaps
- No quantified Kate user data. Terminal-panel praise and Windows-port sentiment were not sourced. KWrite-specific sentiment was not found.

## Large-file handling (multi-GB logs)

### Takeaway
Users expect to open multi-GB logs/CSVs. EmEditor (claims up to 248 GB) and BBEdit are the best-documented performers. VS Code and Sublime are slower. Notepad++ is fine for moderate files but not reliable for multi-GB. Many users fall back to dedicated viewers (klogg, Large Text File Viewer).

### Cited Findings
- EmEditor reportedly supports files up to 248 GB. One tester opened a 30 GB file (slow full load), and another used it on 6.5 GB without trouble. Its design is single-process and multithreaded. — [gHacks](https://ghacks.net/?p=141812); [Codemia](https://codemia.io/knowledge-hub/path/text_editor_to_open_big_giant_huge_large_text_files)
- Notepad++ gets mixed reports: better than Windows Notepad, but some guides leave it off large-file lists because of size limits. — [Petri](https://petri.com/tip-opening-large-files-windows); [Finxter](https://blog.finxter.com/?p=1670118)
- klogg is recommended for searching files from hundreds of MB to tens of GB. Large Text File Viewer opens big files "instantly" with low memory use. — [gHacks](https://ghacks.net/?p=141812)
- Benchmark (Aug 2019, 1.5 GB CSV, 2015 MacBook Pro): BBEdit was about 4x faster to open than VS Code and about 20x faster than Sublime. Excel loaded only 83% of rows in 84 s. Numbers failed after 10 minutes. — [Birchtree](https://birchtree.me/blog/how-long-to-the-major-macos-text-editors-take-to-open-huge-files/)
- A VS Code failure mode: opening a too-large file can crash VS Code, and the Backups (hot-exit) directory then makes it reopen the file on every launch. — [bobbyhadz](https://bobbyhadz.com/blog/vscode-taking-too-much-memory)
- A Microsoft Q&A user reports VS Code feeling "incredibly slow" while using about 1 GB of 128 GB RAM. The user also says `files.maxMemoryForLargeFilesMB` seems deprecated. — [Microsoft Learn Q&A](https://learn.microsoft.com/en-us/answers/questions/2073201/how-to-allocate-more-ram-visual-studio-code)
- An unverified claim puts Sublime's limit at about 2 GB, while G2 reviewers say Sublime stays responsive on large files. — [Codemia (weak)](https://codemia.io/knowledge-hub/path/text_editor_to_open_big_giant_huge_large_text_files); [G2](https://www.g2.com/compare/emeditor-vs-sublime-text)
- Memory comparison threads among free Windows editors exist (AkelPad forum). — [AkelPad forum](https://akelpad.sourceforge.net/forum/viewtopic.php?p=26038)

### Inferences
- Design requirements for a Rust editor:
  - Use a piece table or rope over mmap, so huge files open without full load.
  - Do incremental line indexing in the background and show content first.
  - Turn syntax highlighting and LSP off above a size threshold.
  - Never let session restore re-open a file that crashed the app.
  - Add a read-only/"tail -f" log mode with fast regex search.
- "Opens a 5 GB log without freezing" is a measurable differentiator against VS Code, which users visibly struggle with.

### Gaps
- No recent (2024-2026) controlled benchmarks across editors were found. Zed, Sublime 4 and Notepad++ 8.x large-file numbers were not sourced.

## Session restore of unsaved buffers ("hot exit")

### Takeaway
It is highly valued. Notepad++ users rely on it so much that some keep unsaved tabs for years. Microsoft added it to Windows Notepad by default (2023) and treats it as a core feature. Reliability failures (lost tabs after an update or crash) cause the strongest anger.

### Cited Findings
- Notepad++ users expect browser-like restore of all tabs, including edited unsaved files, after reboot. Unsaved tabs are stored as backup copies named `new 1@<date_time>`. — [Notepad++ Community](https://community.notepad-plus-plus.org/post/41504); [Notepad++ Community](https://community.notepad-plus-plus.org/post/41492)
- One user had 25 unsaved tabs open for 4+ years. A forum regular warned: "you've been lucky to date". — [Notepad++ Community](https://community.notepad-plus-plus.org/post/89419)
- Users report lost tabs after crashes or updates (for example, after v7.6.3). Recovery involves copying `session.xml` or digging through backup folders. — [Notepad++ Community topic 17344](https://community.notepad-plus-plus.org/topic/17344/i-lost-open-tabs-and-probably-files-help/16?page=2); [Notepad++ Community](https://community.notepad-plus-plus.org/post/42308)
- Windows 11 Notepad (v11.2307.22.0, 2023) restores open tabs plus unsaved content by default. State lives in a temp file separate from the documents, and users can switch it off ("When Notepad starts" setting). — [Thurrott](https://www.thurrott.com/windows/windows-11/288011/notepad-and-snipping-tool-are-getting-new-features); [Gigazine](https://gigazine.net/gsc_news/en/20230905-notepad-auto-save); [ElevenForum](https://www.elevenforum.com/t/turn-on-or-off-automatically-save-session-state-for-notepad-in-windows-11.17595/)
- Kate saves splits and layout per named session. — [Planet KDE](https://planet.kde.org/akseli-lahtinen-aks-2025-04-20-how-i-use-kate-editor)

### Inferences
- Must-have: continuous, crash-safe (atomic, journaled) persistence of unsaved buffers. It should be on by default, with visible backup location and recovery UI.
- Losing a buffer is the worst trust failure. Prefer write-ahead snapshots on every idle tick over only on exit.

### Gaps
- No survey quantifies how many users depend on hot exit. Evidence is anecdotal.

## Cloud/browser editors (Codespaces, StackBlitz, Replit, vscode.dev)

### Takeaway
The main pro is zero setup and instant, reproducible environments. The cons are network latency for interactive editing, usage-based cost surprises (Replit's 2025 effort-based pricing backlash is the sharpest case), dependence on connectivity, and reduced capability in pure-browser editors (vscode.dev has no terminal or debugger and limited extensions).

### Cited Findings
- Codespaces: latency is perceptible for interactive editing when the region is not nearest. Billing is by active time times VM size. Gartner reviewers cite long-term cost and lag. — [CloudThat](https://www.cloudthat.com/resources/blog/github-codespaces-vs-local-development-which-is-better); [Gartner Peer Insights](https://www.gartner.com/reviews/market/cloud-development-environments/vendor/github/product/github-codespaces)
- A year-long Codespaces user reports network dependency and "unexpected billing quirks", but judges them minor next to the benefits. — [Dev Genius](https://blog.devgenius.io/github-codespaces-one-year-later-f6f9ead6efd2)
- A 2025 analysis lists zero network latency as the core advantage of localhost. — [SitePoint](https://www.sitepoint.com/the-end-of-localhost-why-remote-dev-environments-are-winning/)
- vscode.dev: no integrated terminal or debugger, and cannot run non-web apps. Themes, keymaps and snippets work, but extensions that need Node OS modules or the shell do not. Python support is very limited (2021 review). — [CodeGuru](https://www.codeguru.com/tools/visual-studio-code-web/); [Towards Data Science](https://towardsdatascience.com/visual-studio-code-is-now-available-as-a-web-app-should-you-care-eb0a7f2766e6/)
- Replit: on 18 June 2025 it moved to effort-based billing. Users reported bills jumping from $20 to $350+ in a day, and one reported $1K in a week on Agent 3 versus $180-200/month before. Replit admitted it "can end up being more expensive over the lifetime of a project". — [The Register, Sep 2025](https://www.theregister.com/2025/09/18/replit_agent3_pricing/); [DEV Community](https://dev.to/sebs/why-i-wont-pay-to-train-your-model-a-developers-farewell-to-replit-ih9)

### Inferences
- A native Rust editor competes here on latency, offline use and predictable (zero) cost. A remote-dev mode (SSH or container, like VS Code Remote) could keep the "zero setup" benefit without the browser limits.

### Gaps
- No StackBlitz/WebContainers sentiment was found in searches. No quantified data on cloud-IDE adoption or offline complaints.

## Windows Notepad: tabs, Markdown, Copilot/AI, backlash

### Takeaway
Tabs and session restore (2023) were broadly welcomed. The 2025-2026 additions (Markdown formatting, tables, Copilot/"writing tools" AI tied to a Microsoft account, a "What's New" startup screen) drew a "bloat/enshittification" backlash. The Markdown link handling then caused an 8.8 CVSS remote-code-execution bug, a concrete example of the risk of feature creep.

### Cited Findings
- May 2025: "lightweight formatting" (Markdown, bold/italic, lists, links, toolbar). Formatting can be disabled in settings. Paul Thurrott called it "the great Notepad controversy of 2025" while defending the change. — [PCWorld](https://www.pcworld.com/article/2799269/windows-11-killed-wordpad-but-notepad-is-absorbing-its-formatting-skills.html); [TWiT](https://twit.tv/posts/tech/defending-notepad-why-microsofts-latest-updates-actually-make-sense)
- Markdown arrived June 2025 and tables in November 2025. In January 2026 came strikethrough, nested lists, streaming AI Write/Rewrite/Summarize, and a "What's New" startup screen. AI features assume a Microsoft account sign-in. The Register headline: Notepad "will now tell you all the ways Microsoft has enshittified it". — [The Register, 22 Jan 2026](https://www.theregister.com/2026/01/22/microsoft_notepad_update/)
- Microsoft later dropped Copilot branding (pen icon, "writing tools", moved under Advanced features), but the AI remains and can be turned off. — [Pureinfotech](https://pureinfotech.com/microsoft-drops-copilot-notepad-windows-11-ai-remains/); [ITdaily](https://itdaily.com/news/software/microsoft-removes-copilot-from-notepad/)
- CVE-2026-20841 (CVSS 8.8, CWE-77 command injection): crafted Markdown links in Store Notepad (versions before 11.2510) could launch unverified protocols and run files after a Ctrl-click. Patched Feb 2026 via the Store. Legacy notepad.exe was unaffected, and there was no known in-the-wild exploitation. — [The Register](https://www.theregister.com/2026/02/11/notepad_rce_flaw/); [Help Net Security](https://www.helpnetsecurity.com/?p=357079); [Windows Latest](https://windowslatest.com/2026/02/12/microsoft-confirms-8-8-rated-security-issue-in-windows-11-notepad-due-to-modernization-efforts-patch-tuesday-fix-rolling-out)

### Inferences
- Users accept quality-of-life features that keep a plain-text identity (tabs, autosave). They reject features that change the product's character (rich formatting, AI, account sign-in, nag screens).
- Lessons for a Rust editor:
  - AI should be opt-in or plugin-based.
  - Never require an account.
  - Show no startup nag screens.
  - Treat link and URI handling in previews as a security boundary.

### Gaps
- No user-survey numbers on Notepad AI adoption or disable rates.

## Other editors: gedit / GNOME Text Editor, BBEdit, Micro, Geany, TextMate

### Takeaway
GNOME Text Editor replaced gedit (Fedora 36+) but is criticized for dropping extensions, snippets and configurability. BBEdit is valued for grep/multi-file search and large-file speed on macOS. Micro is liked for familiar Ctrl-S/C/V shortcuts and mouse support in the terminal. Little 2024-2026 sentiment was found for Geany or TextMate.

### Cited Findings
- GNOME Text Editor: one user calls the missing extension API a dealbreaker ("more polished than functional"). Snippets are missing. It is seen as less configurable than gedit, with possible GTK4 font-rendering differences. Others call it a good replacement. — [GNOME GitLab #348](https://gitlab.gnome.org/GNOME/gnome-text-editor/-/issues/348); [GNOME GitLab #295](https://gitlab.gnome.org/GNOME/gnome-text-editor/-/issues/295); [Fedora Discussion](https://discussion.fedoraproject.org/t/workstation-has-a-new-text-editor-please-test-it/36130)
- gedit is reported as unmaintained and looking for maintainers. — [Ubunlog](https://en.ubunlog.com/gedit-text-editor-no-longer-has-support/)
- BBEdit 15: grep-based multi-file search/replace that reviewers compare to UNIX CLI tools. Multi-file search code was modernized, which sped up search and line processing. Version 15 added a minimap, Cheat Sheets and Text Merge. 15.5.4 adds macOS 26 Tahoe fixes. — [TrustRadius](https://www.trustradius.com/products/bbedit/reviews); [Six Colors](https://sixcolors.com/?p=35143); [Michael Tsai](https://mjtsai.com/blog/2024/01/11/bbedit-15); [TidBITS](https://tidbits.com/?p=66176)
- Micro: Ctrl-S/C/V/Z defaults (rebindable). Mouse drag selects; double/triple-click selects word/line. Has splits and tabs. Positioned between nano and vim. The README calls it a work in progress, with Windows bugs. — [micro README](https://codegraph.jelmer.uk/micro/2.0.15-2/README.md); [MeshWorld guide](https://meshworld.in/blog/how-to/micro-editor-guide/)

### Inferences
- When an editor removes extensibility in the name of simplicity, power users push back. Keep a plugin API even with a minimal core.
- Default CUA keybindings (Ctrl-S, Ctrl-Z, mouse) lower switching cost. That explains Micro's appeal over vim/nano.
- Consolidated "small but beloved" must-have list from all sources:
  - Instant cold start.
  - Low idle memory.
  - Crash-safe hot exit of unsaved tabs.
  - Huge-file open via mmap/rope with automatic degradation.
  - Encoding and line-ending detection and conversion in the status bar.
  - Regex find/replace across files (BBEdit-grade).
  - Column/multi-cursor editing.
  - Macro record/playback.
  - Built-in diff/compare of two tabs.
  - JSON/XML pretty-print and validate.
  - Run-external-command on the buffer (NppExec).
  - Named sessions with split layouts (Kate).
  - Signed, verified auto-update.
  - No forced AI or accounts.

### Gaps
- No 2024-2026 user sentiment was found for Geany or TextMate. Encoding/line-ending control, column mode and macro praise are well known but were not directly sourced in this round. Treat them as unverified for citation purposes. No Reddit threads were surfaced by the search tool.
