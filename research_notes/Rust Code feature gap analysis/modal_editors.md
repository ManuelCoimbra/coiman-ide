# Modal and hackable editors: what Rust Code must offer to win their users

Data pulled 2026-10-30. GitHub stars and reaction counts come from the public GitHub API on that date.

Rust Code status labels:
- HAS: in the feature list supplied with this assignment.
- NO: named as absent in that list.
- PARTIAL: an adjacent feature is listed, but not the full workflow.
- UNKNOWN: not in the list either way. Check the code. No Rust Code source was inspected for these notes.

## Which plugins are near-universal in 2025-2026 Neovim configs?

### Takeaway
Core infrastructure (treesitter, LSP config, lazy.nvim, telescope, gitsigns) appears in the most tracked configs, with which-key, conform, nvim-dap, trouble, todo-comments and textobjects close behind. Rust Code already covers most of the IDE base, so its visible gaps are the small navigation and editing plugins: jump motions, which-key, surround, harpoon, oil, undo tree and search-and-replace.

### Cited Findings
Workflow descriptions below are the researcher's own summaries of well-known plugins, not checked page by page. "Configs" is the count on [Dotfyle's trending list](https://dotfyle.com/neovim/plugins/trending), top 25 only. A dash means the plugin is not in that top 25, not that it is unused.

| Plugin | Stars | Configs | Workflow enabled | Rust Code |
|---|---:|---:|---|---|
| [nvim-treesitter](https://dotfyle.com/neovim/plugins/trending) | not fetched | 2536 | Syntax trees for highlighting and structure | HAS (18 languages) |
| [nvim-lspconfig](https://dotfyle.com/neovim/plugins/trending) | not fetched | 2394 | Per-server LSP setup | HAS (7 servers) |
| [lazy.nvim](https://github.com/folke/lazy.nvim) | 21690 | 2283 | Plugin install, lazy loading, lockfile, UI | PARTIAL (WASM plugin API; no manager listed) |
| [telescope.nvim](https://github.com/nvim-telescope/telescope.nvim) | 19841 | 1911 | Fuzzy pick anything: files, grep, symbols, buffers, git | PARTIAL (quick open, palette, project search; a unified picker is not listed) |
| [fzf-lua](https://github.com/ibhagwan/fzf-lua) | 4476 | - | Same, backed by fzf | PARTIAL |
| [snacks.nvim](https://github.com/folke/snacks.nvim) | 8176 | 960 | Bundle: picker, dashboard, explorer and more | PARTIAL |
| [gitsigns.nvim](https://github.com/lewis6991/gitsigns.nvim) | 7151 | 1786 | Gutter signs, stage hunks, inline blame | HAS |
| [mason.nvim](https://github.com/mason-org/mason.nvim) | 10515 | 1524 | One-command LSP, DAP and formatter install | UNKNOWN (LSP is listed, auto-install is not) |
| [which-key.nvim](https://github.com/folke/which-key.nvim) | 7323 | 1449 | Popup of next possible keys | NO |
| [conform.nvim](https://github.com/stevearc/conform.nvim) | 5392 | 1424 | Format on save, multiple formatters | UNKNOWN |
| [none-ls.nvim](https://github.com/nvimtools/none-ls.nvim) | 3262 | - | Non-LSP tools exposed as LSP sources | UNKNOWN |
| [nvim-dap](https://github.com/mfussenegger/nvim-dap) | 7288 | 1136 | Debugging | HAS |
| [trouble.nvim](https://github.com/folke/trouble.nvim) | 6906 | 1135 | Navigable diagnostics, references and quickfix list | UNKNOWN (LSP is listed, a list panel is not) |
| [todo-comments.nvim](https://github.com/folke/todo-comments.nvim) | 4216 | 1082 | Highlight and jump between TODO and FIXME | UNKNOWN |
| [treesitter-textobjects](https://github.com/nvim-treesitter/nvim-treesitter-textobjects) | 2824 | 1065 | `af`/`if` style objects, swap, jump | HAS (textobjects; swap and jump unverified) |
| [blink.cmp](https://github.com/saghen/blink.cmp) | 6647 | 996 | Fast completion | UNKNOWN (completion engine is not listed, LSP implies some) |
| [harpoon](https://github.com/ThePrimeagen/harpoon) | 9237 | - | Pin a few files, jump by number | NO |
| [oil.nvim](https://github.com/stevearc/oil.nvim) | 6965 | - | Edit a directory as a buffer | NO |
| [flash.nvim](https://github.com/folke/flash.nvim) | 4266 | - | Label jumps, treesitter selection | NO |
| [leap.nvim](https://github.com/ggandor/leap.nvim) | 5027 | - | Two-character label jumps | NO |
| [hop.nvim](https://github.com/smoka7/hop.nvim) | 807 | - | Easymotion-style jumps | NO |
| [neogit](https://github.com/NeogitOrg/neogit) | 5663 | - | Magit-style porcelain | NO |
| [vim-fugitive](https://github.com/tpope/vim-fugitive) | 21815 | - | Git from ex commands, status buffer | PARTIAL (commit panel, branches; no ex interface listed) |
| [lazygit.nvim](https://github.com/kdheepak/lazygit.nvim) | 2380 | - | Launch the lazygit TUI inside the editor | PARTIAL (PTY terminal can host it; one-key integration unverified) |
| [undotree](https://github.com/mbbill/undotree) | 4554 | - | Visual undo branches | NO (local history is adjacent) |
| [mini.nvim](https://github.com/nvim-mini/mini.nvim) | 9611 | - | Bundle of small modules: surround, ai, pairs, files | PARTIAL |
| [overseer.nvim](https://github.com/stevearc/overseer.nvim) | 1944 | - | Task runner | PARTIAL (Cargo test explorer only) |
| [noice.nvim](https://github.com/folke/noice.nvim) | 5825 | - | Floating cmdline, messages and notifications | UNKNOWN (GUI advantage) |
| [persistence.nvim](https://github.com/folke/persistence.nvim) | 1012 | - | Restore session per directory | UNKNOWN |
| [nvim-surround](https://github.com/kylechui/nvim-surround) | 4318 | - | `ys`/`cs`/`ds` | UNKNOWN (brief says "check") |
| [vim-surround](https://github.com/tpope/vim-surround) | 14109 | - | Same, the original | UNKNOWN |
| [vim-visual-multi](https://github.com/mg979/vim-visual-multi) | 4891 | - | Multi-cursor in Vim | HAS |
| [grug-far.nvim](https://github.com/MagicDuck/grug-far.nvim) | 2025 | - | Project search-and-replace buffer with live preview | PARTIAL (project search; replace preview unverified) |
| [nvim-spectre](https://github.com/nvim-pack/nvim-spectre) | 2411 | - | Same, older | PARTIAL |

Other findings:
- Other top-25 items are [LuaSnip 1550, friendly-snippets 1437, nvim-autopairs 1208, lualine 1701, indent-blankline 964, nvim-dap-ui 932](https://dotfyle.com/neovim/plugins/trending). Snippets, autopairs and indent guides are UNKNOWN in Rust Code.
- LazyVim's [editor spec file](https://raw.githubusercontent.com/LazyVim/LazyVim/main/lua/lazyvim/plugins/editor.lua) declares grug-far, flash, which-key, gitsigns, trouble and todo-comments. This shows what the most-used distro's authors picked for its editor layer.
- Distribution and editor stars: [NvChad 28526](https://github.com/NvChad/NvChad), [LazyVim 27765](https://github.com/LazyVim/LazyVim), [AstroNvim 14468](https://github.com/AstroNvim/AstroNvim), [Neovim 103253](https://github.com/neovim/neovim), [Helix 46735](https://github.com/helix-editor/helix), [Vim 41176](https://github.com/vim/vim), [Kakoune 11123](https://github.com/mawww/kakoune), [Spacemacs 24546](https://github.com/syl20bnr/spacemacs), [Doom Emacs 22792](https://github.com/doomemacs/doomemacs), [Lem 3069](https://github.com/lem-project/lem).
- Stack Overflow's 2025 survey is [reported at about 12% Neovim and 13% Vim](https://tech-insider.org/neovim-vs-vim-2026/). This is a secondary source of doubtful quality, and the same site's two articles contradict each other. Check it against the survey before quoting.
- Top open Neovim requests: [upstreaming nvim-treesitter config, #39006, 346 +1](https://github.com/neovim/neovim/issues/39006); [remote-ssh, #21635, 122 +1](https://github.com/neovim/neovim/issues/21635); [unified cmdline, picker and completion interface, #35456, 80 +1](https://github.com/neovim/neovim/issues/35456); [built-in DAP, #11732, 98 +1](https://github.com/neovim/neovim/issues/11732).

### Inferences
- Stars measure mindshare and age, not current use: fugitive has more stars than gitsigns, but gitsigns is the one in Dotfyle's top 25. Prefer config counts where they exist.
- Harpoon, oil and flash have high stars but sit outside this top 25. They are strongly loved, but not provably universal.
- Suggested build order, by evidence strength and small scope: (1) jump motions, (2) which-key popup, (3) surround, (4) persistent undo plus undo tree, (5) diagnostics and quickfix list panel, (6) session restore, (7) search-and-replace with preview, (8) harpoon-style marks, (9) oil-style directory buffer, (10) porcelain git.
- Which-key suits a GUI especially well, and it also solves discoverability for Vim and Helix newcomers.

### Gaps
- No 2025 Neovim user survey results were found. A [Reddit plugin survey by u/PythonPizzaDE](https://dotfyle.com/this-week-in-neovim/50) is mentioned, but its date and results are not confirmed.
- lazy.nvim has no public usage statistics that I found. Dotfyle's total tracked config count was not captured, so no percentages are given. Its sample also skews toward people who publish dotfiles.
- Treesitter and lspconfig star counts were not fetched because the API rate limit was reached.

## What are Helix's most-wanted missing features?

### Takeaway
Helix users most want plugins, folding, auto-reload, sessions, side-by-side diff, a terminal and git features. Rust Code already has most of these, which makes Helix users its easiest audience. Auto-reload and sessions are the items to verify first.

### Cited Findings
Open issues sorted by +1, as `+1 / all reactions`. Older issues have had longer to gather votes.

| Issue | +1 / total | Rust Code |
|---|---:|---|
| [#8675 Steel plugin system](https://github.com/helix-editor/helix/issues/8675) | 521 / 1175 | HAS (WASM plugins, different language) |
| [#1840 Code folding](https://github.com/helix-editor/helix/issues/1840) | 491 / 508 | HAS |
| [#1125 Auto reload on external change](https://github.com/helix-editor/helix/issues/1125) | 355 / 356 | UNKNOWN |
| [#401 Persistent state (session)](https://github.com/helix-editor/helix/issues/401) | 262 / 393 | UNKNOWN |
| [#405 Side-by-side file diff](https://github.com/helix-editor/helix/issues/405) | 256 / 256 | HAS (diff; side-by-side layout unverified) |
| [#1976 Integrated terminal](https://github.com/helix-editor/helix/issues/1976) | 248 / 337 | HAS |
| [#312 Client-server shared buffers](https://github.com/helix-editor/helix/issues/312) | 199 / 210 | UNKNOWN |
| [#13133 Inline git blame](https://github.com/helix-editor/helix/issues/13133) | 181 / 291 | HAS |
| [#2719 Trailing whitespace rendering](https://github.com/helix-editor/helix/issues/2719) | 180 / 206 | UNKNOWN |
| [#5768 File explorer and tree, v3](https://github.com/helix-editor/helix/issues/5768) | 163 / 381 | UNKNOWN |
| [#200 File tree](https://github.com/helix-editor/helix/issues/200) | 156 / 157 | UNKNOWN |
| [#227 VCS support](https://github.com/helix-editor/helix/issues/227) | 122 / 123 | HAS |
| [#5950 Debugging overhaul](https://github.com/helix-editor/helix/issues/5950) | 113 / 267 | HAS |
| [#14544 File watching](https://github.com/helix-editor/helix/issues/14544) | 108 / 241 | UNKNOWN |
| [#2295 Vim-style tabs](https://github.com/helix-editor/helix/issues/2295) | 103 / 105 | UNKNOWN |
| [#4423 Custom typable commands](https://github.com/helix-editor/helix/issues/4423) | 96 / 96 | UNKNOWN |
| [#1596 Soft undo of selections](https://github.com/helix-editor/helix/issues/1596) | 96 / 96 | UNKNOWN |
| [#395 Snippets](https://github.com/helix-editor/helix/issues/395) | 94 / 125 | UNKNOWN |
| [#1565 Code actions on save](https://github.com/helix-editor/helix/issues/1565) | 87 / 87 | UNKNOWN |
| [#5132 Merge conflict resolution](https://github.com/helix-editor/helix/issues/5132) | 81 / 86 | UNKNOWN |
| [#396 Sticky context](https://github.com/helix-editor/helix/issues/396) | 77 / 80 | UNKNOWN |
| [#2806 Universal REPL](https://github.com/helix-editor/helix/issues/2806) | 71 / 78 | PARTIAL (terminal) |
| [#11660 Spellcheck](https://github.com/helix-editor/helix/issues/11660) | 66 / 95 | UNKNOWN |

Other findings:
- A closed request for a [GUI, #39, has 312 +1 / 614 total](https://github.com/helix-editor/helix/issues/39). This is direct evidence of demand for a graphical Helix-style editor. The issue's closed state does not show that a GUI shipped, and I did not read the closing reason.
- [Org mode plugin, #2825, closed, 95 +1](https://github.com/helix-editor/helix/issues/2825) shows some Org interest even among Helix users.
- Persistent undo scores low by title: [#9154 has 6 +1 / 73 total](https://github.com/helix-editor/helix/issues/9154) and [#5608 has 0 +1 / 23 total](https://github.com/helix-editor/helix/issues/5608). Both are closed. Votes may be split across issues, so this is weak evidence against demand.
- The [Helix changelog](https://raw.githubusercontent.com/helix-editor/helix/master/CHANGELOG.md) heads with 25.07.1 (2025-07-18). 25.07 added a directory-exploring picker, EditorConfig support and LSP document colors. It mentions no plugin system in the portion read (first 100000 of 279378 characters).
- [One user left Helix for Neovim in December 2024](https://pawelgrzybek.com/from-helix-to-neovim), citing no package manager or mature plugin system, and missing folding, file tree, snippets and extended git.

### Inferences
- Rust Code's pitch to Helix users: "selection-first editing with folding, terminal, debugger, git and plugins, available now." Helix's last release found here is over a year old, on the changelog read.
- Plugins are demanded by the top issue, but the 1175 total reactions may include dissent or debate. Its 654 non-+1 reactions were not broken down.
- Rust-based WASM plugins are an answer, though Steel fans wanted a scripting language. Compile-free scripting is a possible objection.

### Gaps
- Whether a Helix release or merged plugin work exists after 25.07.1 was not confirmed from the repository's release page.
- Reactions on PRs and issues are mixed in this list. #8675, #13133 and #14544 appear to be implementation proposals rather than plain requests, but I did not open them.

## What do Emacs users praise in magit, org and embark that nothing else matches?

### Takeaway
Evidence gathered is thin and mostly enthusiast opinion. The recurring strengths are Magit's single-hunk and partial-hunk operations, Embark's reversal of command and target, and chained search-to-edit workflows. None of the sources proves these are unmatched. Rust Code has none of the three in full.

### Cited Findings
- Magit: [7249 stars](https://github.com/magit/magit). Rust Code status: PARTIAL (hunks, blame, diff, commit panel, branches; NO full porcelain).
- Slant's community notes [partial-hunk staging and other apply variants such as discard and reverse](https://www.slant.co/versus/37728/1511/~lazygit_vs_magit) as reasons to pick Magit, and ranks it above lazygit. Its main limit is that it requires Emacs.
- The [emacs-bedrock README](https://codeberg.org/T77GIT/emacs-bedrock) calls Magit the best Git interface around. That is personal opinion in a config repository.
- A [Reddit snapshot from December 2025](https://reddit.sentinel-team.org/posts/1pdka62/snapshots/2025-12-05T22%3A20%3A16.701916Z) praises Neogit for trivial hunk staging, so a Neovim clone carries part of the appeal. Another view [prefers lazygit in tmux or inside Neovim](https://www.libhunt.com/r/neogit). I read these two only as search excerpts.
- Embark: [1233 stars](https://github.com/oantolin/embark). Its value is that it ["flips Emacs' action → object ordering"](https://stafforini.com/works/chikmagalur-2021-fifteen-ways-use/) (2021, older). It [ships over a hundred actions for targets such as files, buffers and identifiers](https://packages.gentoo.org/package/app-emacs/embark). Rust Code status: NO (the command palette is action-first).
- [emacs-bedrock describes a chain](https://codeberg.org/T77GIT/emacs-bedrock): ripgrep search, act on results with Embark, export, then edit with wgrep for project-wide replacement. Consult ([1673 stars](https://github.com/minad/consult)) and Vertico ([1895 stars](https://github.com/minad/vertico)) sit in this stack. Rust Code status: PARTIAL (project search; editable results NOT listed).
- Avy: [1959 stars](https://github.com/abo-abo/avy). Rust Code status: NO (jump motions).
- Org: babel has been [part of Org since 7.0](https://list.orgmode.org/87iq2hxi9o.fsf@gmail.com/), and Doom documents [external capture through a shell script and a capture frame](https://gt.emenel.ca/emenel/doomemacs/src/commit/4289df5bc26ad331baf9f90b85f7dece740808a5/modules/lang/org). [nvim-orgmode](https://github.com/nvim-orgmode/orgmode) exists as a port (stars not fetched). Rust Code status: NO.
- Emacs surveys: [the 2022 results](https://emacsconf.org/2022/talks/survey) are the latest found (older). [The 2020 survey had about 7000 participants](https://emacsconf.org/2022/talks/survey), per the results returned by search.

### Inferences
- Target Magit's interaction model rather than its feature list: a status buffer where every line is an actionable object, with discoverable one-key menus. Rust Code's existing hunk and commit panel is the base for this.
- An Embark-style "act on the thing under the cursor or selected in a picker" menu would be a differentiator, and it combines naturally with which-key and the palette.
- Org is a large product by itself. Do not clone it in core. It is better treated as a plugin target, or left out, for a code-focused editor. Users who depend on Org will probably not convert regardless, though no source here measures how many that is.

### Gaps
- No Emacs survey from 2025 or 2026 was found.
- r/emacs and Hacker News threads were not read directly, so verbatim user praise for Org is missing.
- Neogit's parity with Magit could not be checked. The [Neovim discourse thread](https://neovim.discourse.group/t/how-does-neogit-compare-to-magit-are-any-features-missing-from-it/5190) failed with a DNS error.
- Specific Embark demos were not retrieved. The stafforini.com page has only the abstract. The original is on karthinks.com.

## What do users complain about: config churn, breakage, setup cost, LSP pain, Emacs performance?

### Takeaway
Plugin maintenance fatigue pushes some users from Neovim to Helix, but the evidence is individual anecdotes, not trend data. Rust Code's strongest counter-offer is working defaults plus a stability promise, while keeping enough extensibility to avoid the complaint that drove people away from Helix.

### Cited Findings
- [One writer describes hunting for plugins by popularity and recent updates, then moved to Helix](https://world.hey.com/niza/from-vscode-to-neovim-back-to-helix-wait-what-40b1f7d6), also citing Neovim slowness on Windows.
- [A user tired of maintaining a config with almost no plugins left Neovim](https://github.com/helix-editor/helix/discussions/15183) (read only as a search excerpt).
- [A commenter objects that Neovim plugins spawned several language runtimes](https://lemmy.brendan.ie/comment/514678) (search excerpt).
- [A developer cites supply-chain risk and plugin dependencies](https://news.hada.io/topic?id=23632) as reasons to pick Helix on remote servers (search excerpt).
- [A March 2026 Korean article reports plugin and update conflicts as a reason to switch](https://wikidocs.net/blog/@jaehong/8807/). It also flags that Helix does not auto-reload externally changed files, which matters for AI-assisted workflows. The page returned 403 on fetch, so this comes from the search excerpt only.
- Neovim's own direction responds to setup cost. [0.12.0, released 2026-03-29, adds experimental `vim.pack` with a lockfile, but no lazy loading or UI](https://alternativeto.net/news/2026/3/neovim-0-12-has-been-released-with-a-built-in-plugin-manager-major-lsp-and-ui-upgrades). Breaking changes [are listed in its news file](https://neovim.io/doc/user/news-0.12/), including [removal of `vim.diagnostic.disable()`](https://dotfiles.substack.com/p/whats-new-in-neovim-012).
- [Issue #39006, on upstreaming nvim-treesitter's configuration, is Neovim's most +1 open issue at 346](https://github.com/neovim/neovim/issues/39006). The title suggests treesitter setup remains a core pain, but I could not read the body because of the rate limit.
- Emacs GC and startup: [a March 2025 post recommends raising GC thresholds, to 100MB or more with LSP tooling](https://emacsredux.com/blog/2025/03/28/speed-up-emacs-startup-by-tweaking-the-gc-settings/). Users still need to hand-tune this. [EmacsConf 2023 reports that gcmh-mode raises memory use and GC time](https://emacsconf.org/2023/talks/gc/) (older).
- Emacs LSP: [emacs-lsp-booster exists because JSON parsing is slow and Emacs may block when sending data to a busy server](https://lib.rs/crates/emacs-lsp-booster). [Doom keeps a discussion on why it is slow](https://discourse.doomemacs.org/t/83). An [older HN thread](https://brianlovin.com/hn/39119835) has users reporting slow file opens after adding packages.
- Kakoune: the [top open issues are code folding, 58 +1](https://github.com/mawww/kakoune/issues/453), [persistent undo, 17 +1](https://github.com/mawww/kakoune/issues/2021) and [a tutorial, 24 +1](https://github.com/mawww/kakoune/issues/1122). A [commenter who moved to Kakoune reported its own LSP and startup issues](https://lemmy.brendan.ie/comment/514678).

### Inferences
- Rust Code's selling points against these complaints: one native binary, no runtime sprawl, LSP and DAP working on first run, stable TOML, sandboxed WASM plugins with permissions (a direct answer to the supply-chain worry) and no telemetry.
- Auto-reload on external change may now be a must-have, given AI tools editing files in the background. Helix's high vote count (#1125) and the 2026 article both support this.
- Risk: "no plugins needed" alone will not hold users. Helix's experience shows that users leave when missing extras cannot be added.
- Rust Code's seven LSP servers fall short of what Mason-style install offers. An automatic or one-command server installer would address a named pain, though no source here quantifies it.

### Gaps
- No quantitative data on churn or migration exists in the sources found. Reddit r/neovim and r/HelixEditor were not read directly.
- Lem: no user complaints or praise were found beyond its star count.
- Whether nvim-treesitter was archived or rewritten is unverified.

## What do modal users need from a GUI editor's Vim mode?

### Takeaway
Issue trackers for GUI Vim emulations point to three things: correct undo, no input lag, and honest parity with Vim. Jump motions are the top Vim request in Zed. Rust Code's golden-file testing against real Vim directly addresses parity, but it should be published with coverage figures.

### Cited Findings
Zed open issues, reactions by +1:
- [#4930 easymotion, 472](https://github.com/zed-industries/zed/issues/4930), far above any other Vim-titled issue. Also [#59188 flash jump, 17 / 60 total, from June 2026](https://github.com/zed-industries/zed/issues/59188). Rust Code: NO.
- [#10394 surround for HTML tags, 51](https://github.com/zed-industries/zed/issues/10394). Rust Code: UNKNOWN.
- [#4411 live preview for `:%s`, 44](https://github.com/zed-industries/zed/issues/4411). Rust Code: UNKNOWN.
- [#4375 leader mapping, 34](https://github.com/zed-industries/zed/issues/4375). Rust Code: UNKNOWN.
- [#12008 Vim keys inside popups such as file finder and symbols, 31](https://github.com/zed-industries/zed/issues/12008). Rust Code: UNKNOWN. This is the clearest evidence for keyboard-only UI.
- [#17461 `ctrl-p`/`ctrl-n` word completion, 16](https://github.com/zed-industries/zed/issues/17461). Rust Code: UNKNOWN.

VSCodeVim open issues, reactions by +1 (mostly old, but still open):
- [#463 `.vimrc` support, 767](https://github.com/VSCodeVim/Vim/issues/463). Rust Code: UNKNOWN (TOML keybindings are listed, vimrc import is not).
- [#1004 folds opening on idle, 650](https://github.com/VSCodeVim/Vim/issues/1004). A folding integration bug.
- [#2007 `u` undoes the whole stack, 590](https://github.com/VSCodeVim/Vim/issues/2007) and [#1490 undo stack not synced with the host, 317](https://github.com/VSCodeVim/Vim/issues/1490). Native undo gives Rust Code a structural advantage here, if it uses one stack.
- Slowness: [#2021, 318](https://github.com/VSCodeVim/Vim/issues/2021); [#2216, large files, 173](https://github.com/VSCodeVim/Vim/issues/2216); [#3957, degrades over time, 152](https://github.com/VSCodeVim/Vim/issues/3957). Native Rust is a real pitch against this, but verify with benchmarks.
- [#1885 `%` for `def`/`end`, matchit, 100](https://github.com/VSCodeVim/Vim/issues/1885). Tree-sitter could solve this. Rust Code: UNKNOWN.
- [#1782 persistent undo, 76](https://github.com/VSCodeVim/Vim/issues/1782). Rust Code: NO.
- [#2380 saving macros, 64](https://github.com/VSCodeVim/Vim/issues/2380). Rust Code: UNKNOWN. A macro recording UI is NO, per the brief.
- [#3588 multi-cursor documentation for Vim users, 65](https://github.com/VSCodeVim/Vim/issues/3588). Document how Rust Code's Vim mode and multi-cursor interact.
- [#1323 `gj`/`gk` cursor position, 76](https://github.com/VSCodeVim/Vim/issues/1323). Covered if the golden files test wrapped lines. UNKNOWN.

Other findings:
- vscode-neovim's top issues are [folding, 59](https://github.com/vscode-neovim/vscode-neovim/issues/58), [`C-d`/`C-u` viewport, 24](https://github.com/vscode-neovim/vscode-neovim/issues/983) and [undo tree, 11](https://github.com/vscode-neovim/vscode-neovim/issues/459). So even embedding real Neovim leaves host-integration seams. Its project page notes [scrolling, `:e`/`:w`/`:q` and some counts behave differently](https://github.com/jhgarner/vscode-neovim) (a fork's page, read as an excerpt).
- Zed's docs state [ex commands do not take arguments, regex differs from Vim (`$1` replacements, global by default), and macros use Zed's recording system](https://zed.dev/docs/vim). Zed offers `ys`/`cs`/`ds`, sneak, Helix-style `gw` jump labels, `ctrl-w` dock navigation and netrw-style project panel keys there.
- Zed's own comparison, updated May 2026, admits it [may lack full parity in advanced motions, custom macros or rare edge cases](https://zed.dev/compare/vim), that vimrc cannot be imported and that Vim plugins do not run. In January 2025, [Zed planned command history, filename completion and missing commands](https://devclass.com/2025/01/28/rust-powered-zed-editor-plans-vim-mode-improvements).
- Requirements checklist from the brief, with status: registers UNKNOWN; macros UNKNOWN (UI is NO); `:s` UNKNOWN; marks UNKNOWN (harpoon-style is NO); ex commands UNKNOWN; jumplist UNKNOWN; dot-repeat with multi-cursor UNKNOWN; keyboard-only UI navigation UNKNOWN. The golden-file suite should settle the first six. Issue #4411 and #12008 above confirm demand for `:s` preview and keyboard-driven popups. I found no direct source on dot-repeat with multi-cursor.

### Inferences
- Zed is the real competitor: native, Rust, with surround, Helix mode and jump labels already documented. "Has Vim mode" will not differentiate Rust Code. Verified fidelity can: publish the golden-file coverage and any known deviations.
- Ex-commands with arguments, Vim-style regex in `:s` with live preview, and Vim keys in every popup are concrete areas where Zed's docs and tracker show weakness.
- Importing a small `.vimrc` subset (mappings, options) would address VSCodeVim's most-voted issue and Zed's stated limit. Scope it tightly: full Vimscript is unrealistic.
- Maintainers' priority for emulators looks like: undo correctness, latency, then features. Test all three before marketing.

### Gaps
- Direct Reddit and Hacker News testimony from modal users was not obtained, so the evidence rests on issue reactions. Reactions favor long-open issues and tell nothing about users who silently left.
- Rust Code's source was not inspected. All UNKNOWN entries need a code check.
- Tool-call budget and GitHub's unauthenticated API limit ended data collection. `gh` is not installed on this machine, and Neovide and lazygit star counts were not obtained.
