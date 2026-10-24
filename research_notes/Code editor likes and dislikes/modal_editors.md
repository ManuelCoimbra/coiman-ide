# Developer Sentiment: Modal and Terminal-Centric Editors (Neovim, Vim, Helix, Kakoune, Emacs)

Research date: October 2026. About 20 tool calls. Primary sources were preferred. Several areas had few or weak sources; these are flagged under Gaps. Bullets under "Inferences" are analysis, not sourced fact.

## Q1. Neovim: why people switch, pain points, and the plugin ecosystem

### Takeaway
Neovim has a very high admired-to-used ratio. In the 2024 Stack Overflow survey it was "most admired" at about 83%, while only about 14% of 2025 respondents used it. Users praise its speed, keyboard flow, Lua configuration and built-in LSP/tree-sitter. The main pain point is the configuration and plugin burden. Core is now absorbing plugin-manager and LSP-config functions to reduce it: `vim.lsp.config` arrived in 0.11 (2025) and `vim.pack` in 0.12 (March 2026).

### Cited Findings
**Adoption and admiration**
- 2024 SO survey: Neovim was the most-admired IDE at 83% (82.7% in another write-up). VS Code was the most used and most desired at 59% — [Visual Studio Magazine, Jul 2024](https://visualstudiomagazine.com/articles/2024/07/26/so-dev-survey.aspx); [programming.dev summary](https://deddit.petersanchez.com/g/programming@programming.dev/p/fx1834QhgbnWhtFZCK-The-2024-Stack-Overflow-developer-survey-has-bee)
- 2025 SO survey "have used" (IDE section, all respondents): VS Code 75.9%, Vim 24.3%, Neovim 14%, Zed 7.3%. Emacs and Helix appear only as write-ins at 0.1% each — [Stack Overflow 2025 survey, technology](https://survey.stackoverflow.co/2025/technology)
- HandWiki claims Neovim was "most admired" in the 2025 survey for the 5th consecutive year. The 2025 admired percentage could not be verified from the primary page, which was truncated on fetch — [HandWiki: Neovim](https://handwiki.org/wiki/Software:Neovim)
- A 2025 piece on Neovim's roadmap frames the project's driving question as how to compete with VS Code and Zed — [The New Stack](https://thenewstack.io/neovims-future-could-have-ai-and-brain-computer-interfaces/)

**Why people switch / what they love**
- Skills transfer to plain Vim, and the editor is available on almost any Linux box. One user called switching "one of the best career decisions." — [LWN comments on Neovim 0.11, Mar 2025](https://lwn.net/Articles/1015499)
- VS Code felt sluggish as features accumulated, which the author blamed on Electron. Tiling-WM users are drawn toward minimal, keyboard-driven tools — [HackerNoon: transitioning from VS Code to Neovim](https://hackernoon.com/transitioning-from-vscode-to-neovim); [DEV: journey from VS Code to Neovim](https://dev.to/314rate/breaking-the-code-my-journey-from-vs-code-to-neovim-271b)
- One r/neovim user reports fzf-lua is "crazy fast" on Linux. Speed of the picker is part of the appeal — [r/neovim, Feb 2025, "best file picker"](https://redlib.hbubli.cc/r/neovim/comments/1ihcdbp/what_is_the_best_file_picker)

**Pain points**
- Setup was described as a "nightmarish experience." Online guides overlap and contradict each other. You need a plugin manager plus a package manager, and are told you need 5–10 plugins. Many recommended plugins are archived or discontinued. The user had to learn the LSP client/server distinction, then stopped tinkering and never returned — [LWN, Mar 2025](https://lwn.net/Articles/1015499)
- Deprecation churn: the nvim-lspconfig "framework" API was deprecated on Sep 18, 2025, after core added `vim.lsp.config`. This left accumulating deprecation warnings in user configs — [DEV: housekeeping my Neovim configuration, Dec 2025](https://dev.to/m15a/housekeeping-my-neovim-configuration-4kim)
- Distributions are the common workaround. A commenter recommends AstroNvim to avoid configuring advanced features yourself — [LWN](https://lwn.net/Articles/1015499). Distributions also churn: AstroNvim v5 (Apr 2025) significantly changed its plugin set, moving to folke's snacks.nvim — [same thread / AstroNvim migration note](https://lwn.net/Articles/1015499)
- lazy.nvim pins by commit hash, not tag. Guides recommend checking breaking changes before major upgrades — [plugin-research skill guide](https://skills.cat/skills/bityoungjae/marketplace/plugin-research)
- The term "config bankruptcy" did not surface in sources. The underlying phenomenon (giving up on or restarting a config) is described in the LWN thread — [LWN](https://lwn.net/Articles/1015499)

**Core absorbing plugin functions**
- Neovim 0.12.0 (Mar 29, 2026) ships `vim.pack`, a built-in plugin manager with add/update/del/get, a lockfile and hash pinning. It also brings LSP improvements and an opt-in experimental `ui2` message UI that removes "Press ENTER" prompts. Official docs mark `vim.pack` experimental. It lacks lazy.nvim's lazy loading and UI. 0.12 also contains breaking changes — [heise](https://www.heise.de/en/news/Neovim-0-12-with-native-plugin-manager-11241587.html?view=print); [AlternativeTo, Mar 2026](https://alternativeto.net/news/2026/3/neovim-0-12-has-been-released-with-a-built-in-plugin-manager-major-lsp-and-ui-upgrades); [cj.rs blog](https://cj.rs/blog/nvim0.12); [tproger](https://tproger.ru/news/neovim-0-12-0-poluchil-vstroennyj-menedzher-plaginov-vim-pack-i-ul)

**Popular plugins and the need each fills** (sourced where possible)
- Fuzzy finder: telescope.nvim is "a highly customizable fuzzy finder… search and navigate through lists of items." fzf-lua is the speed-focused alternative — [Dotfyle: telescope](https://www.dotfyle.com/plugins/nvim-telescope/telescope.nvim); [r/neovim](https://redlib.hbubli.cc/r/neovim/comments/1ihcdbp/what_is_the_best_file_picker)
- snacks.nvim (folke's QoL bundle: picker, dashboard, notifier and more) appears in many configs. Dotfyle snapshots show 4.7k–6.2k stars — [Dotfyle configs](https://dotfyle.com/alexanderfast/dotfiles-nvim); [AstroNvim v5 adoption via LWN](https://lwn.net/Articles/1015499)
- Completion: nvim-cmp is the most frequently seen, at about 9k stars in a snapshot. Anecdotal migrations go from nvim-cmp + telescope to blink.cmp + fzf-lua, citing speed — [Dotfyle config](https://dotfyle.com/veged/dotfiles-nvim); [r/neovim](https://redlib.hbubli.cc/r/neovim/comments/1ihcdbp/what_is_the_best_file_picker)

### Inferences
- The needs these plugins cover (from general knowledge, not sourced above) map directly to GUI features:
  - lazy.nvim: plugin manager and startup cost.
  - treesitter: highlighting and text objects.
  - oil.nvim: edit the filesystem as a buffer.
  - harpoon: pin 4–5 hot files with instant keys.
  - gitsigns: gutter hunks, stage/reset hunk.
  - which-key: discoverable keymaps.
  - trouble: aggregated diagnostics/quickfix list.
- The trend in Neovim core (`vim.lsp.config`, `vim.pack`, built-in defaults) and the popularity of distributions both show that users want Neovim's power without assembling it themselves. A GUI editor should ship these as defaults.
- Ecosystem churn (telescope to snacks/fzf-lua, cmp to blink) shows that users switch for speed. Picker and completion latency is a competitive feature.

### Gaps
- No primary 2025 SO "admired" figure for Neovim (page truncated). Treat the "5th year most admired" claim as unverified.
- No global Dotfyle ranking or download counts retrieved for oil, harpoon, gitsigns, which-key, trouble, blink.cmp or lazy.nvim. No quantified data on breakage frequency.

## Q2. Helix: praise, complaints, plugin status (Oct 2026)

### Takeaway
Helix is praised as the most productive zero-config terminal editor. LSP, tree-sitter, pickers and multi-cursor work out of the box with a config of about 15 lines. Its #1 complaint is the absence of a plugin system. The Steel (Scheme) plugin PR #8675 is still an unmerged, "S-experimental" draft as of Oct 2026, though it is actively updated and usable through forks. The file-tree complaint was partly answered by a picker-style file explorer in 25.07.

### Cited Findings
- Review (Jul 15, 2026, rated 4.0/5):
  - Praise: tree-sitter, LSP, autocomplete and fuzzy pickers with "zero config files." Setup is "nothing" versus an afternoon of Neovim plugin wiring. Built-in multi-cursor is "the best implementation I have used." Config is about 15 lines across `config.toml` + `languages.toml`, "declarative and boring in the best way." The keymap popup "shows you the next possible keys." Select-then-act often prevented slightly wrong edits.
  - Complaints: no stable plugin system, so "anything the maintainers have not built in simply does not exist." The missing plugins are "the one thing that will send power users back." The selection-first verb order "will fight your muscle memory for a week." There is no file-tree sidebar, but the fuzzy file picker "genuinely replaces it once you trust it." There is no built-in AI or inline completion.
  — [jpk.io Helix review](https://jpk.io/dev-tools/helix-editor-review/)
- Steel plugin PR #8675:
  - Opened Oct 31, 2023 by mattwparas. Still a draft, labeled "S-experimental… won't be merged in its current state."
  - Commits continue through Aug–Oct 2026. Hundreds of hidden comments.
  - Maintainer pascalkuthe: the language choice (Scheme/Steel) is decided. The config system should be revamped to suit Scheme, and the editor should expose generic rather than Steel-specific APIs.
  - Paras (May 2024): usable, but the config API will change.
  - No merge timeline.
  — [GitHub PR #8675](https://github.com/helix-editor/helix/pull/8675)
- Forks with Steel exist: `helix-steel-git` (AUR) and `steelix` (Nix, snapshot 2026-05-21). Third-party plugins target it, for example keel-helix with commits in Oct 2026 — [AUR](https://aur.archlinux.org/packages/helix-steel-git); [MyNixOS steelix](https://mynixos.com/nixpkgs/package/steelix); [Codeberg keel-helix](https://codeberg.org/Keel/keel-helix)
- Plugin design: each plugin is a Steel module exposing functions to Helix, with one shared engine instance — [Steel discussion #78](https://de.github.com/mattwparas/steel/discussions/78)
- Helix 25.07 (Jul 15, 2025):
  - File explorer (`<space>e`) as a telescope-like picker that browses directories.
  - Tree-sitter integration replaced by the `tree-house` crate.
  - Inline LSP color swatches and a command-line rewrite. 195 contributors.
  — [x-cmd blog summary](https://www.x-cmd.com/blog/250720); [openSUSE changelog](https://src.opensuse.org/pool/helix/commits/branch/factory)
- Usage is small: Helix had 0.1% write-in "have used" in SO 2025 — [SO 2025](https://survey.stackoverflow.co/2025/technology)

### Inferences
- Helix shows that strong built-in defaults win admiration even without extensibility. The ceiling on its growth is extensibility (AI, file tree, custom tools).
- Lesson for a Rust GUI editor: ship batteries-included and design the extension API early. Helix's maintainers favor a generic API, so the language binding is secondary.
- Keymap divergence from Vim is a real migration cost. A GUI editor should offer a Vim-compatible mode first and selection-first (Helix/Kakoune-style) as an option, not as the only mode.

### Gaps
- No quantitative r/HelixEditor sentiment data. Could not confirm whether any 2026 Helix release (for example 26.x) changed the plugin situation beyond the PR status above.

## Q3. Kakoune and selection-first editing

### Takeaway
Kakoune pioneered selection-first, multi-selection editing ("object then verb"), which Helix adopted. It shows the effect of a command before acting, and regex-driven multi-selection replaces many macros.

### Cited Findings
- Kakoune is built around selections, which are oriented inclusive ranges with anchor and cursor. Some text is always selected, even a single character. Motions create selections, so there is no separate select step. Multiple selections are the central interaction model, with regex-based split/select. It is keystroke-competitive with Vim — [Red Hat blog](https://www.redhat.com/en/blog/kakoune-vi-text-editor); [Fedora package description](https://packages.fedoraproject.org/pkgs/kakoune/kakoune); [LinuxLinks](https://linuxlinks.com/kakoune-implements-vi-keystrokes-text-editing-language-model)
- The selection-first benefit from a Helix user: you see what will be affected before acting, which prevents wrong edits — [jpk.io](https://jpk.io/dev-tools/helix-editor-review/)

### Inferences
- In a GUI, selection-first maps naturally to visual feedback. Primitives worth adopting: multi-cursor "select all regex matches in selection," "split selection on regex," and "keep/remove selections matching."

### Gaps
- No 2024–2026 usage numbers or community sentiment for Kakoune specifically.

## Q4. Emacs (incl. Doom, Spacemacs): org-mode, magit, extensibility, complaints

### Takeaway
Emacs is loved for unlimited extensibility and killer apps, chiefly Magit and org-mode. Complaints center on single-threaded performance, elisp and the learning curve. Doom Emacs is the "fast, curated" distribution, and Spacemacs the "more features, slower" one.

### Cited Findings
- Magit aims to be a complete Git porcelain, so experienced users do "almost all" daily VCS work inside Emacs. Staging is the headline feature:
  - Staging a single hunk is as trivial as staging a whole file, which beats `git add --patch`.
  - Discard and revert also work at hunk granularity, which Git's own porcelain lacks.
  - It is "consistent and efficient," and users say it taught them what Git can do.
  - Current manual: v4.7.0 (2015–2026).
  — [Magit manual](https://magit.vc/manual/magit.html); [Magit about](https://magit.vc/about/)
- Emacs User Survey 2022: about 6,600 responses (about 1,000 partial) from 115 countries. Magit's popularity was highlighted, with its lead narrowing versus 2020. The 2020 survey had about 7,000 responses — [EmacsConf 2022 survey talk](https://emacsconf.org/2022/talks/survey)
- Performance architecture: Eli Zaretskii (2023) says Emacs "is still a single-threaded Lisp machine" and that concurrency would need an internals redesign. The display engine is significantly more expensive than the old one. Long-standing complaints say Gnus and network modes hang Emacs — [emacs-devel via yhetil](https://yhetil.org/emacs/ZQcc6739eWDcXj8o@ACM/t); [emacs-devel 2005](https://lists.gnu.org/r/emacs-devel/2005-06/msg00234.html)
- Doom vs Spacemacs:
  - Doom FAQ: Doom is lighter and starts faster because packages are managed outside Emacs. Spacemacs has more features and checks packages at startup.
  - One user measured an unoptimized Spacemacs startup at about 10 s versus Doom at 1.2 s.
  - Doom's lazy loading can cause a pause the first time a command is used. Doom docs are "quite sparse" compared with Spacemacs.
  — [Doom FAQ](https://gt.emenel.ca/emenel/doomemacs/src/commit/5ae5d80c68cc7388ce7e843f51eb96f1769d792a/docs/faq.org); [Spacemacs to Doom migration](https://valrus.codeberg.page/2018/08/19/Spacemacs_To_Doom); [Clojurians log](https://clojurians-log.clojureverse.org/practicalli/2023-03-03); [Practicalli](https://practical.li/doom-emacs/introduction/why-doom-emacs/)
- Emacs appeared only as a 0.1% write-in in SO 2025 "have used," far smaller than Vim or Neovim — [SO 2025](https://survey.stackoverflow.co/2025/technology)

### Inferences
- Magit's lessons for a GUI Git panel:
  - Every Git object (file, hunk, line range, commit) is a selectable item with single-key verbs (s/u/k/c).
  - Transient popups show available flags.
  - Operations are fully keyboard-driven and discoverable.
  - Hunk- and line-level stage/discard is first-class.
- Emacs's single-threaded lesson: run LSP, indexing, Git and search off the UI thread. This is where a Rust editor can beat Emacs on its weakest axis.
- Doom and Spacemacs show the same pattern as LazyVim and AstroNvim: curated defaults plus a leader-key menu (SPC prefix with which-key-style hints).

### Gaps
- No 2024–2026 Emacs survey results found. No sourced 2024–2026 org-mode sentiment data. Org-mode's appeal (outlining, agenda, literate programming) is general knowledge only.
- No quantified elisp or learning-curve complaint data.

## Q5. What these users dislike in VS Code-like editors; Vim-mode quality expectations

### Takeaway
Modal-editor users dislike Electron-era sluggishness and mouse dependence. Above all, they dislike Vim emulation that is "almost right." Edge cases matter: counts, `.` repeat, registers, macros, undo granularity. The two strategies have known trade-offs:
- Embedding real Neovim (vscode-neovim) gives perfect semantics but causes sync and undo bugs.
- Native emulation (VSCodeVim, Zed) integrates better but has a long tail of gaps.

Zed's approach tests its native emulation against headless Neovim golden files. That is the best-practice model to copy.

### Cited Findings
- Zed Vim mode (Jun 2024 blog):
  - Built natively in Zed's `vim` crate rather than embedding Neovim. Embedding would mean discarding or porting Zed's rope/CRDT/SumTree/render/async foundations and building features like collaboration twice.
  - Core mismatch: Vim addresses characters, Zed addresses positions between characters. This causes newline edge cases.
  - Hard-to-match semantics: `5dj` vs `d5j`, `.` repeat, counts on `i`/`a`.
  - Gaps at the time: macros missing, basic registers only, jumplist and changelist off. More than 20 PRs landed in about 2 months (marks, surround, `gn`/`cgn`).
  - Users offered a $500 bounty for `r`/`R` replace modes.
  - Vim mode composes with editor-native features (`gl` multi-cursor, `]x`/`[x` tree-sitter node selection).
  - Tests run keystrokes against a headless Neovim to produce "golden" JSON expectations.
  — [Zed Decoded: Vim](https://zed.dev/blog/zed-decoded-vim)
- Zed's own comparison (May 2026) admits its Vim mode "may lack full parity" in advanced motions, custom macros and rare edge cases — [zed.dev/compare/vim](https://zed.dev/compare/vim)
- Zed planned Vim-mode improvements (Jan 2025). A decade-long Neovim user found "Zed is very fast" with easier config, with caveats about early bugs — [DevClass, Jan 2025](https://www.devclass.com/development/2025/01/28/rust-powered-zed-editor-plans-vim-mode-improvements/1626128)
- vscode-neovim (embeds real Neovim):
  - Issue: after `u`, the buffer desyncs from what VS Code shows. Recovery requires reopening the buffer.
  - VS Code's undo stack is not loaded into Neovim, so persistent undo breaks.
  — [vscode-neovim undo desync issue (Lightrun mirror)](https://lightrun.com/answers/vscode-neovim-vscode-neovim-buffer-desync-on-undo--redo); [persistent undo issue](https://lightrun.com/answers/vscode-neovim-vscode-neovim-persistent-undo)
- VSCodeVim has 7M+ downloads. A commenter claims a Vim extension inside VS Code can't match native Vim performance (anecdotal) — [DevClass](https://devclass.com/2025/01/28/rust-powered-zed-editor-plans-vim-mode-improvements)
- Electron sluggishness is a cited reason for leaving VS Code — [HackerNoon](https://hackernoon.com/transitioning-from-vscode-to-neovim)

### Inferences (design recommendations for a Rust VS Code-style GUI editor)
- **Vim mode as first-class, native, test-verified:**
  - Copy Zed's golden-file differential testing against headless Neovim.
  - Prioritize the parts users notice: counts on operators and motions, `.` repeat, text objects (`ciw`, `da(`, `it`), registers (named, `"+`, `"0`), macros (`q`/`@`), marks, jumplist (`C-o`/`C-i`), `:s` with ranges, visual-block, and undo granularity (one insert session = one undo).
  - Make all editor commands bindable from normal mode. Let `:` reach the command palette.
- **Avoid split-brain buffers:** if you ever embed Neovim, the undo/sync bugs above are the cost. Native emulation over a single buffer model is preferable.
- **Keyboard-first everything:**
  - Fuzzy pickers for files, symbols, grep, buffers, commands, diagnostics and git; they must be instant.
  - which-key style leader menus.
  - Harpoon-style pinned files.
  - Oil-style editable directory buffer.
  - Trouble-style diagnostics list.
  - Gitsigns-style hunk staging.
  - Magit-style keyboard Git panel.
- **Tree-sitter text objects and syntax-node selection** (Helix `A-o`/`A-i`, Zed `]x`), usable from Vim mode.
- **Optional selection-first mode** (Helix/Kakoune) and native multi-cursor primitives (select regex in selection, split, align).
- **Batteries included, zero-config LSP and tree-sitter,** like Helix. Declarative config (TOML) for the 90% case.
- **Scripting/plugin API designed early,** because no plugins is Helix's #1 complaint. Lock API stability to avoid Neovim-style churn, and offer pinning and lockfiles.
- **Latency and responsiveness:** avoid Emacs-style single-thread stalls by keeping language servers, search and Git asynchronous. Keystroke latency is a selling point against Electron.

### Gaps
- No quantitative head-to-head of VSCodeVim vs vscode-neovim vs Zed Vim mode (bug counts, coverage, latency) was found.
- No primary Reddit or HN threads retrieved on Vim-emulation complaints. No keystroke-latency measurements for these editors were retrieved in this pass.
