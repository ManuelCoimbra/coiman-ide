# Cross-cutting code editor expectations 2025-2026, mapped to Rust Code

Status tags: **[HAS]** per the brief, **[PARTIAL]**, **[MISSING]**, **[REPO]** checked in the Rust Code repo on 2026-10-30.

Evidence notes for the report writer:
- Search-summary items were not always opened at source. Pages actually opened: Stack Overflow 2025 AI page, IT Brief on the 2026 survey, Zed remote and dev container docs, Zed compare page, The Register on the co-author incident, CSA GlassWorm note.
- The JetBrains Code With Me post returned HTTP 403 on fetch. Its facts come from the search summary only.
- Vendor claims are labelled. Performance numbers are mostly not reproducible.

## 1. Remote development

### Takeaway
Remote is table stakes for professional use. VS Code remains the most reliable baseline, and Zed shows that a native editor can split the UI locally from a headless server. Rust Code has none of it **[MISSING]**, and an SSH plus WSL server split is the largest functional gap in this theme.

### Cited Findings
- Zed splits the work: UI, model calls, Tree-sitter and unsaved-change storage stay local. Source, language servers, tasks and terminal run remotely. — [Zed remote docs](https://zed.dev/docs/remote-development)
- Zed SSH targets: macOS Catalina or later, Linux x86_64/arm64, Windows x86_64/arm64. WSL has dedicated open actions. — [Zed remote docs](https://zed.dev/docs/remote-development)
- Zed's stated limits: the `zed` command does not open files from the remote terminal; directories over 100,000 files, such as `/` or `~`, open poorly; the server does not use the local proxy configuration. — [Zed remote docs](https://zed.dev/docs/remote-development)
- Locally installed Zed extensions propagate to the server, so their language servers run there. — [Zed remote docs](https://zed.dev/docs/remote-development)
- Zed dev containers exist but are "still in development". They need Docker or Podman in `PATH`, and `devcontainer.json` changes do not trigger a rebuild, so users must kill the container manually. — [Zed dev container docs](https://zed.dev/docs/dev-containers)
- Microsoft's Remote extensions are licensed only for official VS Code builds, so Cursor rebuilt its own remote layer, and the tutorial says that rebuild is where gaps appear. — [Tech Insider, secondary](https://tech-insider.org/?p=23323)
- Fork users fell back on the community `open-remote-ssh` extension from Open VSX (2024, older, user-reported). — [Cursor forum](https://forum.cursor.com/t/ssh-not-working-in-latest-update/1201?page=3)
- An HPC guide calls Cursor's remote-over-SSH less mature than VS Code's while supporting the same Dev Containers flow. — [Illinois Tech GRC guide](https://grc.iit.edu/docs/hpc-tutorials/ide/overview)

### Inferences
- Best today: VS Code for breadth; Zed is the model to copy for a native architecture.
- Open differentiators for Rust Code: an auditable open-source server, an explicit consent prompt before any server binary is uploaded, no proxying through a vendor, and clean handling of huge directories. Zed's 100,000-file limit shows that indexing is a weak point.
- Suggested order: SSH, then WSL (one protocol), then Dev Containers. Codespaces and Gitpod can be reached through SSH if they expose it, which I did not verify.

### Gaps
- No verified 2026 data on JetBrains Gateway reliability, Codespaces or Gitpod (Ona) status.
- No survey numbers on share of developers using remote development.
- Specific current Remote-SSH failure modes were not confirmed.

## 2. Collaboration

### Takeaway
Real-time collaboration is shrinking as a market: JetBrains is retiring Code With Me. Zed is the showcase, and Live Share stays available. Rust Code is **[MISSING]** both collaboration and in-editor PR review. PR review is likely the more valuable first step, though this is inference.

### Cited Findings
- JetBrains is sunsetting Code With Me: 2026.1 is the last supported IDE version, no new features follow, public relay shuts down after at least Q1 2027, and Enterprise renewals end. — [JetBrains, via search summary, page returned 403](https://blog.jetbrains.com/platform/2026/03/sunsetting-code-with-me/)
- Zed markets built-in multiplayer as "Google Docs for code"; it describes Live Share as mature but needing setup and an extension (vendor framing). — [Zed compare](https://zed.dev/compare/vscode)
- Zed says it runs its own team inside the editor. — [Zed blog](https://zed.dev/blog/zed-is-our-office)
- No evidence was found that Live Share is being retired. — search result, absence of evidence only.

### Inferences
- A JetBrains exit suggests limited demand or return on maintaining a relay. Do not build a relay early.
- A privacy-consistent design would be peer-to-peer or self-hosted, end-to-end encrypted, session-scoped and off until invoked.
- PR/MR review through a user-supplied token fits consent-gated network use.

### Gaps
- No verified data on the GitHub PR or GitLab MR extensions, or on usage of pair-programming features.
- Zed's user-preference claims come from small blogs and were excluded as anecdotal.

## 3. AI expectations and rejections

### Takeaway
Developers use AI heavily and distrust it heavily. The clearest backlash pattern is not AI itself but AI imposed by default without consent: trailers, training and pricing changes. "No AI by default, every call visible, verifiable diffs" is a defensible position for Rust Code **[MISSING: all AI, by design]**. Pure no-AI risks exclusion from the majority workflow, so optional BYOK and local models are the natural fit.

### Cited Findings
Surveys:
- SO 2026 (30,000+ responses, 169 countries): 66% use coding assistants or agents, 63% chat tools, 26.2% automated agent workflows, 17.2% no AI tools. — [IT Brief](https://itbrief.co.uk/story/stack-overflow-survey-shows-developers-trust-ai-with-caveats)
- SO 2026: nearly half trust AI output they can verify easily; 6.6% would trust it for important decisions; 78.7% rate attribution important or very important. — [IT Brief](https://itbrief.co.uk/story/stack-overflow-survey-shows-developers-trust-ai-with-caveats)
- SO 2026: AI features cited at about 2% as a reason for recommending technologies. — [IT Brief](https://itbrief.co.uk/story/stack-overflow-survey-shows-developers-trust-ai-with-caveats). Search results also showed 2.3% in another IT Brief version; primary not checked.
- SO 2026 is secondary coverage; primary survey site was not opened.
- SO 2025: 84% use or plan to use AI (76% prior year); 51% of professionals use it daily; about 60% favorable, down from over 70%. — [SO 2025](https://survey.stackoverflow.co/2025/ai)
- SO 2025: 33% trust accuracy, 46% distrust; 66% cite "almost right" solutions; 45.2% say debugging AI code takes longer. — [SO 2025](https://survey.stackoverflow.co/2025/ai)
- SO 2025 agents: 31% use at work; 37.9% have no plans; 81% have security or privacy concerns; 87% accuracy concerns. — [SO 2025](https://survey.stackoverflow.co/2025/ai)
- JetBrains 2025 (24,500+ responses, 194 countries): about 85% regularly use AI; 62% rely on an assistant, agent or AI editor; 44% report AI fully or partly adopted in workflows; 19% of users save 8+ hours a week; 23% worry about code quality. — [The Register](https://www.theregister.com/2025/10/21/massive_jetbrains_dev_survey), [heise](https://heise.de/-10771317). Primary not opened.

Backlash:
- VS Code added a "Co-authored-by: Copilot" trailer by default. It appeared even with Copilot unused or chat disabled, and survived a user rewriting the message. The reviewer apologized and the fix made it opt-in, scheduled for 1.119. — [The Register](https://www.theregister.com/2026/05/04/microsoft_reverses_ai_credit_grab/)
- Version conflict: The Register dates the change to 1.110 (March 2026). Other sources say 1.110 shipped the setting off, and an April 16 PR enabled it, in 1.117 or 1.118. The second account likely describes the default flip, but this was not confirmed. — [It's FOSS](https://itsfoss.com/news/vs-code-credits-copilot-for-human-work/)
- HN reaction reported as 1,458 points and 805 comments (search summary, not opened at source). — [It's FOSS](https://itsfoss.com/news/vs-code-credits-copilot-for-human-work/)
- GitHub: from April 24, 2026, Copilot Free, Pro and Pro+ interaction data trains models unless users opt out. Business and Enterprise are exempt. — [heise](https://www.heise.de/en/news/Only-with-opt-out-GitHub-will-train-Copilot-models-with-user-data-in-the-future-11226787.html?view=print), [The New Stack](https://thenewstack.io/github-copilot-interaction-data/). Coverage is pre-effective; private repo handling is reported inconsistently.
- Cursor's June 2025 move to a usage pool brought reports of $10-20 daily charges, followed by an apology and refunds. — [TechCrunch](https://techcrunch.com/2025/07/07/cursor-apologizes-for-unclear-pricing-changes-that-upset-users/)
- A comparison site reports a June 2026 Copilot credit backlash. — [pricepertoken, low authority](https://pricepertoken.com/coding-assistants/compare/cursor-vs-github-copilot)

Best-in-class patterns:
- Zed has `"disable_ai": true`, which turns off the Threads Sidebar, Agent Panel, Edit Prediction and Inline Assistant. — [Zed docs](https://zed.dev/docs/ai/configuration)
- A past bug let edit predictions show despite the flag, so the switch needs tests. — [Zed commit](https://git.secluded.site/zed/commit/899bc8a8fd9c82565cc1b37659b226587b699929)
- Zed supports BYOK and local paths: Ollama, LM Studio, OpenAI-compatible servers. — [Zed docs](https://zed.dev/docs/ai/configuration)
- Zed's Agent Panel and Inline Assistant work in remote sessions. — [Zed remote docs](https://zed.dev/docs/remote-development)

### Inferences
- Must-have list if AI ships: off by default; a hard kill switch; BYOK; Ollama and LM Studio; MCP; project rules files; per-hunk diff approval; checkpoints; per-request token and cost display; a pre-send preview of context; attribution showing sources. The verification and attribution numbers support diff review and attribution; the rest is market inference.
- Never write AI trailers or anything else into commits without an explicit choice.
- The 78.7% attribution figure and low trust are an opening: an "every network call is logged and visible" pane is a claim few can match.

### Gaps
- No sourced comparison of next-edit quality, checkpoint implementations, MCP or rules-file support across editors. The checkpoint search returned nothing useful.
- 2026 trust question differs from 2025's, so the two are not directly comparable.

## 4. Accessibility

### Takeaway
VS Code has the most documented screen reader support. Zed publicly concedes it has fewer accessibility tools. This is a real opening, but Rust Code should not claim it yet: **[REPO] AccessKit is only an eframe feature flag, and the custom editor view has no AccessKit code of its own.**

### Cited Findings
- VS Code pages the text for screen readers, tested with NVDA, VoiceOver and Orca. Its screen reader mode disables features such as folding and minimap. Docs note limited macOS and Linux support. This doc is an archived copy, possibly old. — [VS Code docs mirror](https://vscode-docs-arc.readthedocs.io/en/latest/editor/accessibility/)
- A blind engineer on NVDA said VS Code had only been good for about a year and that he still sometimes uses Notepad++ (date not checked). — [Uses This](https://usesthis.com/interviews/austin.hicks/)
- Zed admits fewer accessibility tools than VS Code. — [Zed compare](https://zed.dev/compare/vscode)
- A GPUI fork's source comments say Zed sets no a11y APIs on any UI element. This is a fork and may be stale for upstream Zed. — [open-gpui](https://docs.rs/crate/open-gpui/0.1.0/source/src/window/a11y.rs)
- AccessKit bridges to NSAccessibility, UI Automation and AT-SPI. — [gpui-ce a11y source](https://docs.rs/crate/gpui-ce/0.3.0/source/src/window/a11y.rs)
- AccessKit text editing work was tested through egui branches on Windows; word-movement mismatches were tied to an egui bug (undated, older). — [AccessKit PR #145](https://github.com/AccessKit/accesskit/pull/145)
- egui issue #4396 (April 2024) asked for docs on labelling custom widgets. — [goodfirstissue mirror](https://www.goodfirstissue.org/emilk/egui/issues/4396)
- Zed extension offers high-contrast and colorblind-friendly Modus themes. — [Zed extensions](https://zed.dev/extensions/modus-themes)
- **[REPO]** `Cargo.toml` enables `accesskit` on eframe 0.37. A case-insensitive search for `accesskit` in `.rs` and `.toml` files matched only that line. `src/editor_view.rs` handles input directly, so the widget's text exposure must be tested. — local repo.

### Inferences
- Assume the buffer is invisible to screen readers until proven otherwise with NVDA, VoiceOver and Orca.
- High-value path: expose a text node with caret and selection for the visible region, status announcements, focus order, keyboard-only coverage, a high-contrast theme, and colorblind diagnostics that do not rely on color. Recruit blind testers before making claims.
- Best today: VS Code. A native Rust editor with a working screen reader would be unusual, but I cannot prove it would be unique.

### Gaps
- No AppleVis or blind-forum feedback specific to Zed, JetBrains or Cursor was found.
- Current Zed upstream screen reader state and current egui TextEdit status are unverified.
- No sourced comparison for font scaling or keyboard-only coverage.

## 5. Settings and onboarding

### Takeaway
Zed's flow is the practical bar: settings editor plus file, VS Code import, and keymap choice at setup. Rust Code has layered TOML, keybinding TOML and VS Code theme import **[PARTIAL]**, with no settings UI, first-run flow, profiles or sync **[MISSING]**.

### Cited Findings
- Zed offers to import key VS Code settings during setup, with a later command, `Zed: Import VS Code Settings`. — [Zed migration guide](https://zed.dev/docs/migrate/vs-code)
- Zed provides a VS Code keymap choice at onboarding plus a Keymap Editor; the guide indicates extensions and keybindings are not imported. — [Zed migration guide](https://zed.dev/docs/migrate/vs-code)
- Zed's AI docs refer to a searchable Settings Editor alongside the settings file. — [Zed docs](https://zed.dev/docs/ai/configuration)
- Zed's compare page says its VS Code rival's configuration can become time-consuming (vendor framing). — [Zed compare](https://zed.dev/compare/vscode)

### Inferences
- Minimum set: a generated settings UI over the TOML schema, a first-run page (keymap, theme, import, consent choices), VS Code settings and keybinding import, JetBrains and Vim presets.
- Privacy-respecting sync could be Git- or file-based or end-to-end encrypted with no vendor account. No source validated demand for this design.
- Rust Code's consent model should be visible in first-run, because it is the product pitch.

### Gaps
- Nothing verified on VS Code Settings Sync privacy, profiles across editors, or JetBrains import flows.
- No user research on TOML versus UI preference.

## 6. Performance and resources

### Takeaway
Direction is consistent: native editors beat Electron. Rigorous 2025-2026 numbers are scarce, and most published figures conflict. Rust Code can win by publishing a reproducible benchmark, not just by being fast. **No Rust Code numbers were measured here.**

### Cited Findings
| Metric | Zed | VS Code | Source and quality |
|---|---|---|---|
| Startup | under 1 s | 3-5+ s with extensions | [Zed, vendor, no methodology](https://zed.dev/compare/vscode) |
| Startup, folder open | 0.60 s | 1.29 s | [Search summary, blog](https://tech-insider.org/zed-vs-vscode-2026/); attribution among results unverified |
| Typing latency | under 10 ms | can exceed 50 ms under load | [Zed, vendor](https://zed.dev/compare/vscode) |
| Typing latency | 2 ms / 8 ms | 12 ms / 18 ms | [Blogs, conflicting](https://dev.to/truongandev/zed-vs-vs-code-in-2026-is-it-worth-switching-17gm) |
| RAM | about 600 MB | 3-4 GB or more | [Zed, vendor](https://zed.dev/compare/vscode) |
| RAM | 222 MB; 180 MB idle | 3,549 MB; 650 MB | [Blogs, conflicting](https://tech-insider.org/zed-vs-vscode-2026/) |

- BBEdit opened 1GB+ CSVs 4x faster than VS Code and 20x faster than Sublime, per one macOS user. — [Birchtree](https://birchtree.me/blog/how-long-to-the-major-macos-text-editors-take-to-open-huge-files/)
- An HN commenter reported about 20 GB memory use in Zed for find and replace on a 1 GB file (anecdote). — [HN mirror](https://hn.svelte.dev/item/45362425)
- A secondary guide says VS Code enters a large-file mode above 50 MB, disabling highlighting and bracket matching. Not checked against Microsoft docs. — [fast.io](https://fast.io/resources/cline-file-size-limit/)
- gopls returns an error for semantic tokens above 100,000 bytes, so LSP limits apply regardless of editor. — [gopls commit](https://go.googlesource.com/tools/+/dbc874762811d2b12635393a0de7e8870828e3a5)
- Zed's remote docs warn about 100,000+ file directories. — [Zed remote docs](https://zed.dev/docs/remote-development)

### Inferences
- Treat ratios as indicative, not exact. The 222 MB versus 600 MB Zed spread shows how much test setup matters.
- Rope **[HAS]** helps editing but does not settle search memory. Test 1 GB find and replace, as the Zed anecdote suggests.
- Publish a repo with fixed hardware, project size, and a keyboard-to-photon method. Include Zed, VS Code, Sublime.
- Rust Code uses eframe's `glow` backend **[REPO]**. Idle repaint and battery use should be measured because immediate-mode UIs can repaint often. This is a technical inference, not a sourced finding.

### Gaps
- No Pavel Fatin-style 2025-2026 latency test found (his work predates this window).
- No battery measurements found. No Sublime numbers in usable form. No file watcher or monorepo data.

## 7. Built-in tools and workflow features

### Takeaway
This theme was the least well sourced. I found no survey ranking these features, so the list below is a status inventory, not demand evidence.

### Cited Findings
- A JuliaCon 2026 talk by a Zed support engineer lists debugger, notebook, test, REPL and plotting among items assessed for Julia in Zed. — [JuliaCon 2026](https://pretalx.com/juliacon-2026/talk/GQ8NM3/)
- Zed says many tools are built in instead of relying on extensions, with fewer extensions overall. — [Zed compare](https://zed.dev/compare/vscode)
- **[REPO]** "snippet" appears only in four LSP files (`src/lsp_ui.rs`, `src/app/lsp.rs`, `core/src/lsp/convert.rs`, `core/src/lsp/client.rs`). This implies LSP snippet handling, and no user snippet UI or file was found by that search. — local repo.
- **[REPO]** Search for `multi_root` and `workspace_folders` in `.rs` and `.toml` found nothing. This suggests no multi-root under those names; not proof. — local repo.

Status (from the brief, no external source):
| Feature | Status |
|---|---|
| Markdown preview, Mermaid | [MISSING] |
| Notebooks | [MISSING] (not listed) |
| REST client, DB client, Docker/K8s panels | [MISSING] (not listed); plugin candidates |
| Spell check | [MISSING] |
| Snippets | [PARTIAL] LSP only |
| TODO tracking, bookmarks, project manager | Not listed; verify |
| Multi-root | Likely [MISSING]; verify |
| Tasks/problem matchers | [PARTIAL] Cargo/test explorer; general tasks not listed |

### Inferences
- Candidate core items: markdown preview (with Mermaid), user snippets, offline spell check, bookmarks, TODO scan, tasks with problem matchers, multi-root. These need no network, so they fit the pitch.
- Candidate WASM plugins: REST, DB, Docker, K8s. They would exercise capability permissions, with user-visible network grants.
- Offer Git-file-based HTTP request files in place of a cloud account. This is a design inference.

### Gaps
- No source found for Mermaid, REST, DB, Docker or spell-check demand or per-editor status.
- Zed multi-root and markdown state not confirmed.

## 8. Security and supply chain

### Takeaway
Extension marketplaces are now an active attack surface. Rust Code's WASM capability model, signed registry and lockfile **[HAS]** answer exactly the failure modes seen in GlassWorm. This is the strongest verifiable differentiator, provided install-time and runtime enforcement holds and workspace trust is added **[not listed, verify]**.

### Cited Findings
GlassWorm:
- Disclosed by Koi Security in October 2025 as the first self-propagating VS Code extension worm in the wild. — [CSA](https://labs.cloudsecurityalliance.org/research/csa-research-note-glassworm-supply-chain-developer-toolchain/)
- Wave 1 (Oct 17, 2025): 13 Open VSX plus 1 Marketplace extension, about 35,800 downloads. Wave 2: 16 days after "contained", 3 extensions, about 9,700 downloads. — [CSA](https://labs.cloudsecurityalliance.org/research/csa-research-note-glassworm-supply-chain-developer-toolchain/), [BleepingComputer](https://bleepingcomputer.com/news/security/glassworm-malware-returns-on-openvsx-with-3-new-vscode-extensions)
- Wave 4 (late January to mid-March 2026): an estimated 433 components across Open VSX, Marketplace, GitHub, npm, including 72 Open VSX extensions and about 151 GitHub repos. CSA itself calls this an estimate. — [CSA](https://labs.cloudsecurityalliance.org/research/csa-research-note-glassworm-supply-chain-developer-toolchain/)
- Techniques: transitive `extensionPack` or `extensionDependencies`, runtime remote payloads, account takeover, invisible Private Use Area Unicode, Solana memo C2 with a Google Calendar fallback. Targets include tokens, SSH keys, CI secrets and 49 wallet extensions. — [CSA](https://labs.cloudsecurityalliance.org/research/csa-research-note-glassworm-supply-chain-developer-toolchain/)
- CSA notes version pinning alone cannot stop runtime-fetched payloads. — [CSA](https://labs.cloudsecurityalliance.org/research/csa-research-note-glassworm-supply-chain-developer-toolchain/)

Others:
- Wiz (October 2025): over 500 extensions examined across both marketplaces, 550+ validated secrets, including publishing tokens that could push updates to every install; VS Code auto-updates by default. — [Wiz](https://www.wiz.io/blog/supply-chain-risk-in-vscode-extension-marketplaces)
- Microsoft added publisher secret scanning, sandboxed scans of packages and periodic rescans in June 2025, and revoked reported tokens. — [Wiz](https://www.wiz.io/blog/supply-chain-risk-in-vscode-extension-marketplaces)
- TigerJack: at least 11 malicious extensions stealing code, mining crypto, installing backdoors. — [CSO Online](https://www.csoonline.com/article/4074948/threat-actors-are-spreading-malicious-extensions-via-vs-marketplaces-2.html)
- Vicarius argues extensions run unsandboxed with full user privileges and that verified badges may only confirm domain ownership (vendor opinion). — [Vicarius](https://www.vicarius.io/articles/malicious-vs-code-extensions-and-the-new-developer-supply-chain-threat)

### Inferences
Mapping attacks to Rust Code controls:
- Runtime payload fetch and C2: blocked if network capability is denied by default.
- Secret theft: blocked if filesystem and environment capabilities are scoped.
- Account takeover update: signed registry helps only if signing keys are separate from publish tokens; lockfile stops silent update.
- Transitive delivery: show the whole dependency permission set before consent.
- Permission growth on update: require a re-prompt.
- Invisible Unicode: render and warn on Private Use Area and bidi control characters in the editor and in diffs. This protects users even from malicious repositories, and I found no evidence for how widely editors do it.
- Workspace trust: LSP, tasks, debugger configs and project settings can execute code on folder open. Rust Code needs a trust gate. Verify current behavior.
- Run registry publishing with mandatory 2FA and a public revocation list.

### Gaps
- No verified 2026 change to VS Code workspace trust or extension signing.
- No confirmation of GlassWorm activity after March 2026.
- No independent audit of Zed extension sandboxing was located.

## 9. Platform issues

### Takeaway
Linux Wayland and IME remain hard even for Zed. Rust Code has basic IME handling **[REPO]**, but egui-level gaps and no bidi evidence mean CJK, RTL and Wayland should be treated as untested. These are good tests to run before any cross-platform claim.

### Cited Findings
- Zed: blurry UI and buffer fonts at 120% on KDE Plasma Wayland, worked around by maximizing or fullscreen. — [Zed #33464](https://github.com/zed-industries/zed/issues/33464)
- A Zed fix for fractional blur (#33087) shipped in v0.195.4, and a related change was reverted for causing size problems. — [Zed commit log](https://git.secluded.site/zed/commits/v0.195.4?page=9), [revert](https://git.secluded.site/zed/commit/d609931e1c27e9c42aa18ce328808bbce3149b64.patch)
- Zed fixed IME popups appearing outside the window on scaled Wayland, because Wayland uses logical pixels and X11 does not. — [Zed #37600](https://git.secluded.site/zed/commit/57c6dbd71e483646cd0409894547e97f664ebed3)
- Electron apps show the tradeoff: Obsidian users report fcitx works only with Wayland disabled, which then blurs under fractional scaling. — [Obsidian forum](https://forum.obsidian.md/t/ime-fcitx-doesnt-work-when-wayland-is-enabled/79941)
- egui TextEdit dropped a committed Pinyin character unless composition had begun (0.24.1, older; fix PR #3768 submitted, merge unconfirmed). — [egui #3766](https://github.com/emilk/egui/issues/3766)
- **[REPO]** `src/editor_view.rs:787-788` handles `egui::ImeEvent::Preedit` and `Commit`. No `bidi` match was found in `.rs` or `.toml` files. — local repo.
- **[REPO]** The root `eframe` line sets `default-features = false` with `accesskit`, `clipboard`, `default_fonts`, `links`, `glow`, and lists neither `wayland` nor `x11`. — local repo.

### Inferences
- Check the Linux build first: the eframe line may need `wayland` and `x11` unless they are set elsewhere. I did not inspect target-specific dependencies or build on Linux.
- Test matrix: Wayland on GNOME and KDE at 125%, 150%, 175%; Fedora and Ubuntu with fcitx5 and IBus; Windows Japanese, Chinese and Korean IMEs; macOS Japanese; Arabic and Hebrew display and caret movement; font fallback; ligatures.
- egui text layout may limit RTL and shaping. This was not verified; test before committing to a design.

### Gaps
- No usable source on RTL or ligatures in Zed, VS Code or egui. Two targeted searches returned irrelevant results.
- No current egui RTL or shaping status. No sourced Windows/macOS/Linux parity comparison.
