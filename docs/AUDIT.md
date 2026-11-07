# Coiman IDE: audit against the quality review

Audit of the code as it stood at commit `f23397a` (still named Rust Code), against
`Coiman IDE.md`. Each claim says how it was found:

- **app**: reproduced in the running release build, with a screenshot and the app log
- **code**: read in the source, not run
- **grep**: searched for and not found

Date: 2026-11-06.

## 1. Confirmed bugs

| # | Area | Problem | How found | Cause |
|---|------|---------|-----------|-------|
| B1 | Command palette (§13) | Typing `git` lists *Change Indentation*, *Go to Definition* and others before *Git: Commit* | app | `palette.rs` `fuzzy_match` is a yes/no subsequence test with no score. Results stay in declaration order. Nucleo is already a dependency, used only for Quick Open. |
| B2 | Palette mouse (§13, §15) | A click on a row to the right of its label does nothing. The list is narrower than the popup, with its scroll bar in the middle | app | Only the `Button::selectable` label is clickable, not the row. The `ScrollArea` is as wide as its widest row, not the popup. |
| B3 | Markdown preview (§3) | Emoji and CJK show as boxes | app | Font fallback: see B6 |
| B4 | Markdown preview (§3) | Images show as `[image: Logo]` | app, code | `core/src/markdown.rs` never loads images, by design |
| B5 | Markdown preview (§3) | Relative links (`docs/other.md`) can't be clicked | app, code | `md_preview.rs` only opens `http`, `https` and `mailto` |
| B5b | Markdown preview (§3) | `**bold with `code` and a [link](…)**` shows its raw backticks and brackets | app | The hand-made parser has no nested spans |
| B5c | Markdown preview (§3) | Table columns are a few characters wide. Text wraps a word a line and spills under the next row | app | `egui::Grid` with wrapping labels and no column widths |
| B5d | Markdown preview (§3) | A blockquote holding a list and two paragraphs comes out as one line | app | `Block::Quote` holds one paragraph of spans |
| B5e | Markdown preview (§3) | Code blocks have no highlighting. `<details>` and other HTML show as text | code | The parser keeps the language name but nothing uses it |
| B6 | Unicode (§18) | Box drawing, emoji and CJK show as empty boxes in the Markdown preview and other UI text. Emoji and CJK show as boxes in the editor too. Box drawing in the editor and terminal is correct: it uses Hack's glyphs, which fill the line height so they join. | app (`b_md.png`, `a_tree.png`) | Three causes, see 1a |
| B6b | Unicode (§18) | CJK takes one cell instead of two, so its glyphs overlap. Combining marks take a cell of their own. | code | `buffer.rs:1114` `char_width` returns 1 for every character except tab. The editor squeezes each glyph into `char_width` cells (`editor_view.rs:1643`). |
| B7 | HTML diagnostics (§14) | `<li>` left open, `<div><span></div></span>` and an unclosed `<p>` give no problems | app | Nothing checks markup when no HTML language server is installed |
| B8 | Explorer keys (§8) | Delete and F2 do nothing in the explorer. Delete and Rename are only in the context menu | code, grep | `explorer.rs` reads no keys. Fixed in Phase 2: see section 9 |
| B9 | Image viewer (§10) | No wheel zoom or drag to pan, only Fit and 100%. `.tiff`, `.svg` and `.avif` don't open in it, though `tif` does. No EXIF orientation. A GIF shows its first frame. Downscaled pictures use `NEAREST` and look jagged | code | `web.rs` `ImageView`. A 17000×400 PNG did open (app). |
| B10 | Image viewer and status bar | With an image showing, the explorer still selects the previous file and the status bar names its language | app | The image is drawn over a document, not as its own tab |
| B11 | Recent (§7) | Only folders. No way to remove one, clear them, or see recent files. Missing folders aren't flagged | code | `core/src/recent.rs` is a 10-entry list with only `add`. Fixed in Phase 2: see section 9 |

### 1a. Deeper causes (2026-11-06)

**Fonts.** The coverage of each font was read from its `cmap` table (script `cmap.py` in the session scratchpad):

| Font | Box drawing | Emoji | CJK |
|------|-------------|-------|-----|
| Hack (built in, monospace) | yes | no | no |
| Ubuntu-Light (built in, UI text) | no | no | no |
| NotoEmoji, emoji-icon-font (in `epaint_default_fonts`) | no | yes | no |
| Segoe UI Symbol | yes | yes | no |
| Segoe UI Emoji | no | yes (colour) | no |
| Yu Gothic, Microsoft YaHei | yes | no | yes |

1. egui 0.37 moved its emoji fonts behind the `monochrome_emoji_fonts` feature. `Cargo.toml` turns on only `default_fonts`, so the app has no emoji font at all.
2. `install_fonts` adds Segoe UI Symbol only when `pump_symbol_font` loads it (`profile.rs:28`). That happens only for symbol-typing language profiles and *Insert Symbol*. Everywhere else, UI text falls from Ubuntu-Light straight to egui's icon font.
3. No CJK font is ever added.

Fix with fonts already on this computer, no download. Colour emoji would need epaint's `color_fonts` feature. Its `vello_cpu` crate is already in `Cargo.lock`.

**Markdown parser.** Running `markdown::parse` on a test file (`core/examples/md_probe.rs`) also shows these failures:

- A list item's lazy continuation line becomes a separate paragraph.
- Setext headings (`===`) stay as text.
- Indented code blocks become paragraphs.
- Hard line breaks (two trailing spaces) are lost.
- Reference links (`[x][r]` with `[r]: url`) stay as text.
- Nested emphasis, code and links (B5b) aren't parsed.

JSON and YAML fences do keep their text: that part of §3 works. These are the CommonMark gaps a hand-written parser accumulates. `pulldown-cmark` passes the CommonMark spec tests.

**HTML.** `tree-sitter-html` is already a dependency, used for highlighting. Its tree for the test file (`core/examples/html_tree_probe.rs`) holds every needed signal:

- `erroneous_end_tag` for the stray `</span>`
- `element` nodes with a `start_tag` and no `end_tag` for `<span>` and `<b>`
- the same for `<li>` and `<p>`, which HTML closes implicitly, so these should be hints rather than errors
- an `ERROR` node for the unterminated `class="x>`

A checker can be a tree walk with HTML's lists of void elements and optional end tags. That needs no new dependency and no Node.

## 2. Missing features

| # | Section | Feature | Notes |
|---|---------|---------|-------|
| M1 | §13, §11 | Git: Clone | grep: no command |
| M2 | §11 | Git author check (`user.name`, `user.email`) and a form to set them | `core/src/git.rs` reads them only for commits |
| M3 | §2 | HTML auto close tag, and `=""` after an attribute name | Emmet exists. Auto-close covers only brackets and quotes. |
| M4 | §2 | Java and C# getters, setters, constructor and `toString` | grep: none. A Java or C# language server's code actions give these when one is installed. |
| M5 | §2 | C and C++ null-check hints | Only a rule in `likely.rs`. clang-tidy through clangd gives more. |
| M6 | §4 | Maven: run goals, refresh, dependencies | Only detection of `pom.xml`, for run, test and icons |
| M7 | §6 | CSS font-family preview | Done in Phase 2: see section 9 |
| M8 | §17 | Frame-rate limit setting | grep: none. egui repaints on demand already, so idle use should be low. Not measured yet. |
| M9 | §9 | Running VS Code extensions | There's no extension host. Only data is imported (themes, snippets, grammars). Code extensions need Node.js and the `vscode` API: see §9 below. |
| M10 | §12 | Guided introduction | There's a Welcome page, but no tour |
| M11 | §23 | Coiman IDE name and icon | Everything still says Rust Code |

## 3. Partly done

| # | Section | What exists | What's missing |
|---|---------|-------------|----------------|
| P1 | §21 Run | One path, `run_program`, runs the active file through `run::plan` and saves first. Ctrl+F5 runs the chosen configuration. | When a configuration is pinned, Shift+F10 runs it instead of the file. The tab button and the run marker both call `run_program`, so they do the same. Not yet reproduced: which file the user saw fail. |
| P2 | §16 Rainbow brackets | On by default (`editor.bracket_pair_colorization`), in Settings | Done in Phase 2: see section 9 |
| P3 | §2 Indentation | `insert_newline` keeps indentation and splits braces. Tabs or spaces follow the file and `.editorconfig`. | Language-specific indent after `:` (Python) and outdent on `}` and `</tag>`: not checked |
| P4 | §15 Completion | Click, arrows, Page Up and Down. It scrolls to the selection. | No hover highlight. Rows are painted, not widgets. Not reproduced with a language server yet. |
| P5 | §5 Colours | Swatches in completion (`completion_color`). Light colours (`light_colors.rs`). | Text drawn on the colour's own background, readable either way |
| P6 | §19 IntelliSense | Hover, signature, rename, references, code actions, call hierarchy | Auto import depends on the server. Not re-tested in this audit. |
| P7 | §8 Shortcuts | Keymap with a Keyboard Shortcuts editor and conflict list | Done in Phase 2: see section 9 |
| P8 | §22 Explorer | IntelliJ-like tree, nesting, compact folders, file watcher | Name search inside the tree, a remembered folder per file dialog |

## 4. Working (seen in the app this session or earlier)

- Quick Open by file name (nucleo, ranked)
- Opening a 17000×400 PNG
- Markdown preview: headings, paragraphs, lists nested to 3 levels, rules, code blocks with JSON and YAML text visible, https links
- Islands layout, tool rails, Search Everywhere, Settings, logs, theme schedule and backgrounds: commits `90cae26` to `f23397a`

## 5. Dependencies and decisions

| Item | Needs |
|------|-------|
| Markdown rework (B3–B5e) | A real parser: `pulldown-cmark` (crates.io, pure Rust, no network code). Highlighting can reuse `syntect`, which is already in. Needs your OK to download. |
| CJK and emoji (B6) | Fonts already installed (see 1a). `unicode-width` 0.2.2 is already in `Cargo.lock`, for B6b. |
| HTML diagnostics (B7) | `tree-sitter-html`, already in (see 1a). No download. |
| Maven (M6) | `mvn` or the project's `mvnw` on PATH. jdtls for Java code actions. Installing either is a download that needs your OK each time. |
| VS Code extensions (M9) | A real extension host is Node.js plus a large share of the `vscode` API. That's a separate product. Proposal: keep importing data, list unsupported extensions by reason (`main` entry with no data parts), and use language servers instead. |
| GitHub repo (§26) | `gh` login. It publishes the code: done last, with your confirmation. |
| Rename (§23) | The config folder, binary and crate names change. Settings need copying from `rust-code` to the new folder. |
| §28 usage | No usage figure is visible to this session, so the 95% rule can't be checked. No release will be claimed from it. |

## 6. Complexity and order

S: under a day. M: one to three days. L: more.

**Phase 1, in this order:**

1. B1 and B2, palette ranking and clicks: S
2. B6, font fallback: S
3. B3–B5e, Markdown on `pulldown-cmark`: M
4. B7, HTML checker: M
5. P4, completion mouse: S
6. B9 and B10, image viewer: M
7. P1, run: S. Needs the failing case.
8. P3, indentation: S

**Phase 2:** M3 and P5 (S), B8 and P7 (S), B11 (S), M7 (S), P2 (M), P8 (M).

**Phase 3:** M1 and M2 (M), M6 (L), M4 and M5 (M), M9 compatibility report (M).

**Phase 4:** M10 (M), top-bar search (S), M8 (S), M11 rename and icon (M), §26 GitHub.

**Order changes and why:**

- Fonts move before Markdown, because the preview's emoji depend on them.
- The palette goes first: it's critical and small.

## 7. Checklist

Each row is a reported problem: its cause, what changed, the tests and the result.

| Item | Cause | Files | Change | Tests | Result | Left |
|------|-------|-------|--------|-------|--------|------|
| B1 palette ranking | Yes/no subsequence filter, so results stayed in declaration order | `core/src/finder.rs` (`rank`), `src/palette.rs` | Commands, plugin commands, pickers and settings are ranked by nucleo's score. Words match in any order, and matched letters are highlighted. | unit test `ranks_the_closest_label_first`; app: `>git` lists the 13 Git commands first | fixed | |
| B2 palette clicks | Only the label was clickable. The list was as wide as its widest row. | `src/palette.rs` | The whole row is one click target, with hover and selection painted behind it. The list fills the popup. | app: a click in the empty right part of a row ran it (`GitShowHistory`), and so did one after scrolling (`CloseSplit`) | fixed | |
| B6 fonts | No emoji feature. Symbol font only for symbol profiles. No CJK font. | `Cargo.toml`, `core/src/fonts.rs`, `src/app/profile.rs`, `src/app/config.rs`, `src/app/mod.rs` | egui's `monochrome_emoji_fonts`. Segoe UI Symbol, Yu Gothic and Malgun Gothic (or the macOS and Linux equivalents) are read off the UI thread at start and put behind both font families. | app: box drawing, emoji and 日本語 drawn in the preview, editor, outline and breadcrumbs (log: `fallback fonts: symbols, cjk, hangul`) | fixed | Emoji are one colour. `⚠️` shows a dotted circle for U+FE0F. CJK sits a little high. About 30 MB more memory. |
| B6b widths | `char_width` was 1 for everything | `core/src/buffer.rs`, `core/src/unicode.rs` | Widths from `unicode-width`. Invisible and bidi characters keep a cell for their mark. | unit test `wide_and_combining_characters_take_their_cells`; app: `日本語` takes 6 cells | fixed | Combining marks not yet tried in the app |
| B3–B5e Markdown preview | Hand-written parser: no nested spans, no images, local links not followed, tables in a Grid with no widths, a quote held one paragraph | `core/src/markdown.rs`, `core/src/highlight.rs` (`snippet`), `src/app/md_preview.rs`, `src/app/web.rs`, `src/app/svg_preview.rs` | The parser is pulldown-cmark, with GitHub's tables, task lists, strikethrough and footnotes. Spans carry every style at once. Lists and quotes hold blocks. A small reader shows common HTML tags. The preview lays out each paragraph as one text, with clickable links. Pictures beside the file load off the UI thread, SVG included. Table columns are sized to their text, never below their longest word. Code blocks are highlighted with syntect. | 8 unit tests: nested spans, CommonMark cases, lists and quotes, tables, images, code and HTML, anchors, odd input, column widths, link paths. App (`md_fixture.md`, 100 lines, and the project README, 517 lines): every element above drawn. The `#tables-and-code` link scrolled. `docs/other%20page.md` opened. PNG and SVG shown. The web picture shown as a link, and the missing picture says so. | fixed | Bold is only a brighter colour: the UI font has no bold weight. CJK sits a little high. A footnote's text loses its label. Pictures on the web are never fetched (privacy). TypeScript and some other languages have no colours in code blocks, because syntect's default grammars lack them. |
| B7 markup diagnostics | Nothing checked markup without an HTML language server, and that server doesn't check tags anyway | `core/src/markup.rs`, `src/app/markup.rs`, `src/app/lsp.rs`, `src/app/mod.rs`, `core/src/settings.rs`, `src/app/settings_ui.rs` | HTML is read with tree-sitter-html, which was already a dependency, and XML with quick-xml. They report: tags never closed, closed in the wrong order or never opened; broken tags marked where they break, such as an unclosed quote; repeated attributes; `<div/>`. Tags HTML closes by itself are information. Problems go to the editor and the Problems panel. The setting `editor.markup_problems` turns it off. | 6 unit tests: open, misnested and stray tags; a broken tag; repeated attributes; a good page with `<` inside script and style; UTF-16 columns; XML. App: squiggles, inline messages and Problems entries for each case. Typing `</b>` cleared its error. A `.csproj` mismatch was reported at line 3. | fixed | Attribute names aren't checked against HTML's list. Templates (Jinja, Handlebars) in `.html` may confuse it. XML gives only its first error. |
| P4 completion mouse | Pressing on the list took keyboard focus from the editor, and `completion_after_typing` closed the list on lost focus before the click was released. So a click picked nothing. There was no hover highlight. | `src/app/lsp.rs` | The list keeps its rectangle. It stays open while the pointer is on it. A mouse pick hands focus back to the editor. The hovered row is highlighted. | App with rust-analyzer: `s.` opened 60+ methods. After scrolling the wheel, the hovered row (`match_indices`) was highlighted, a click inserted `match_indices(…)`, and the next key typed went to the editor. | fixed | No automated test: it's pointer and focus behaviour |
| B7b markup, the rest | Attribute names were never checked. Template tags broke the parse. XML stopped at its first error. | `core/src/html_attributes.rs`, `core/src/markup.rs` | Each element's attributes come from the WHATWG index, with global attributes, `data-*`, `aria-*`, events and framework syntax allowed, and obsolete ones noted. Template tags are blanked byte for byte before parsing, so positions hold. Where a file has template blocks, tag balance becomes information. Duplicates inside a templated tag aren't reported. Tags still open at the end of a file are reported, not called unreadable. XML matches its own tags, so it recovers, and names repeated attributes. | 10 unit tests, including templates (Jinja, Handlebars, EJS) and XML recovery. App: `clas` and `hreff` warned, `bgcolor` noted. The Jinja page shows no false error and notes the `<div>` and `<span>`. The XML file shows 5 problems, including "`x` is set twice". | fixed | Attribute values aren't checked. SVG and MathML attributes aren't checked. A template that builds tag names out of variables can still confuse it. |
| B9, B10 image viewer | Only Fit and 100%, `NEAREST` filtering, no EXIF orientation, a GIF's first frame only, `tiff` missing, no size limit while decoding. The explorer and status bar followed the document under the picture. | `src/app/image_view.rs` (new, moved from `web.rs`), `src/app/md_preview.rs`, `src/app/workbench.rs`, `src/app/mod.rs` | Zoom by buttons, keys, Ctrl+wheel and pinch about the pointer. Pan by dragging and the wheel. Linear filtering and mipmaps when smaller, nearest when larger. EXIF orientation. GIF and WebP animation (1000 frames at most). Decoding capped at 1 GB. Pictures past the GPU texture size are scaled down, and say so. `.tif`, `.tiff` and `.jfif` added. AVIF, HEIC, JXL and PSD explained, not dumped as hex. The explorer marks the picture, and the status bar says Image. The Markdown preview's pictures are turned too. | 5 unit tests: fit, zoom about the pointer, pan limits, frame timing, loading (PNG, too wide, broken, AVIF). App: alpha PNG on the checkerboard; zoomed to 195% and dragged; 16-pixel art at 931% crisp; EXIF-6 JPEG and TIFF upright; the GIF animating; 20000 x 50 PNG fitted at 5%; broken PNG and AVIF messages; status bar and explorer following. | fixed | SVG opens as text with its own preview, not in this viewer. Ctrl+wheel and pinch weren't driven by the test script, only the keys. A picture past the texture limit wasn't seen in the app: this GPU holds 20000 pixels. |
| P1 run | Choosing a configuration with Ctrl+F5 also pinned it, silently. From then on Shift+F10, the title bar's Run and the ▶ beside `main` all ran that configuration, never the file in front. Ctrl+F5 didn't save first. | `src/app/cargo.rs`, `core/src/commands.rs`, `src/keymap.rs`, `src/app/dispatch.rs`, `src/app/workbench.rs`, `core/src/file_actions.rs` | **Run: Run Current File** (Ctrl+Shift+F10, as in IntelliJ) always runs the active file, and so do the ▶ beside `main` and package.json's Scripts button. Shift+F10 still runs what the title bar shows. Pinning says so in the status bar, and how to undo it. Every path saves first, and configuration runs are logged. | App, reproduced first: after Ctrl+F5 picked "Serve", Shift+F10 on hello.py and hello.c ran Serve. After the fix: Shift+F10 runs Serve and says so; Ctrl+Shift+F10 ran hello.py; back on Current File, Python, Python in a subfolder, C (gcc), Java (javac) and JavaScript (node) each ran their own file; a changed file was saved before it ran. | fixed | No unit test: it's command routing. Programs run in the project folder (VS Code's default), not the file's folder; there's no setting to change that yet. |
| P3 indentation | Enter knew only `{ ( [`. It didn't handle Python's `:`, HTML tags or other languages' blocks, didn't dedent after `return`, and left spaces on a blank line. Backspace removed one space of an indent. Nothing moved a closing `}`, `end` or `else:` back. | `core/src/indent_rules.rs` (new), `core/src/buffer.rs`, `src/app/files.rs` | Indent rules per family: brackets, Python, markup, Ruby, Lua, shell, Elixir, YAML, and plain for prose. Enter goes in after an opener, out after Python's `return`, splits `{\|}` and `<p>\|</p>`, and goes on with `/** */` comments. Typing a closer, `</`, `else:` or a finished `end` word moves its line back, never deeper. Backspace in indentation goes back to the tab stop. A blank line keeps no spaces. | 9 new unit tests holding 60-odd cases (rules, Enter per language, blank lines, tabs, closers and words, Backspace). App, by keystrokes, compared with the saved files: Python (def, if, return, else, pass), HTML (ul, li, /ul), Ruby (def, end), C (`{` split), JS (Backspace a whole level), JS (typed `}` back to its opener), and Markdown, where `**bold**` doesn't start a list. All matched. | fixed | SendKeys can't type AltGr characters into this app: it sends Left Ctrl+Alt. A real AltGr+7 does type `{` (checked), so it's a test-tool limit, not a bug. JSX tags indent by brackets only. |

## 8. Phase 1 review (2026-11-06)

A high-effort code review of `f23397a..aca3f48` found 10 problems. All are fixed:

1. **Security:** Markdown pictures at `//host/share/x.png` or `\\host\share` were read over SMB as soon as the preview opened, which on Windows sends the user's NTLM hash. Network paths are now refused, for pictures and links.
2. Animated GIF and WebP decoders had no memory limits, so a frame could allocate its declared size before the 1 GB check. They now get the same limits as stills.
3. Markdown pictures weren't fitted to the GPU's texture size, and every file's pictures were kept. They're fitted now, and dropped when the preview moves to another file.
4. `*ptr = 1;` followed by Enter continued as a ` * ` comment. A `*` line now continues only inside a `/*` comment.
5. Links starting with `/` went to the drive root. They now start from the open folder, as on GitHub.
6. `<meta property>` (Open Graph), `part`, `exportparts` and two `<video>` attributes were flagged as unknown. They're known now, with the rest of RDFa.
7. A word-ending bracket or quote after `end` lost its auto-closed pair. Only a space or `;` finishes the word now.
8. The palette made a new matcher every frame. It now keeps one per thread.
9. The preview laid out the whole document every frame. Blocks out of view now take their remembered height.
10. The void-element list was copied in two places. It's shared now.

After the fixes:

- `scripts/check.ps1` passes: 453 core tests and 27 app tests.
- The Markdown, HTML, image, indentation and palette app scripts ran again with the same results as before.

## 9. Phase 2

| Item | Cause | Files | Change | Tests | Result | Left |
|------|-------|-------|--------|-------|--------|------|
| M3 tags and quotes | Typing in markup only paired brackets and quotes | `core/src/auto_tags.rs` (new), `core/src/buffer.rs`, `core/src/settings.rs`, `src/app/files.rs`, `src/app/settings_ui.rs` | `>` adds the end tag, `</` finishes the innermost open element's, and `=` adds `""` after an attribute name. Modes: HTML (void elements, script and style left alone), XML (every element) and JSX (only where an expression can start; fragments). Two settings turn them off. | 5 unit tests (start tags, void and self-closing, comments, quoted `>`, script code, JSX type arguments and arrows, `</` with comments and script, `=` cases, modes), and one buffer test typing a whole tag. App, by keystrokes, compared with the saved files: HTML `<div class=box">` gave `<div class="box">`, then `</div>` and the Enter split, a closed `<p>` and an open `<br>`; XML `</` became `</root>` and moved back; TSX left `Array<string>` alone and closed `<Card>`. | fixed | No completion of attribute names or values; that needs the HTML language server or Phase 2's IntelliSense work. Vue's `<template>` follows HTML rules, not JSX. |
| M3b suggestions and Vue | Attribute names and values weren't suggested without the HTML server, and Vue files weren't checked or given Vue's directives | `core/src/html_complete.rs` (new), `core/src/html_attributes.rs`, `core/src/markup.rs`, `src/app/lsp.rs` | When no ready server serves an HTML, template or Vue file, typing and Ctrl+Space offer element names, attributes (the element's own first, as snippets with the cursor in the quotes, without those already there), and values for 38 enumerated attributes. In Vue they also offer directives, `:` bindings, `@` events, built-in components and SFC block attributes. The markup checker reads `.vue` with Vue's rules (self-closing tags, `setup`, `scoped`, `module`, `generic`; Pug skipped) and `.svelte` with Svelte's (its `{...}` blocks and expressions blanked, attribute values kept as values). The status bar names Vue, Svelte, SVG, XAML, MSBuild and the template languages. | 4 suggestion tests, a Vue and a Svelte checker test. App: a good Svelte component showed no problems; a broken one showed its open `<b>` (as a note, inside `{#each}`) and `clas`. App, by keystrokes (with a real AltGr for `@`), compared with the saved files: `<in` + Enter + ` ty` + Enter + `ch` + Enter gave `<input type="checkbox">`; in Vue, `<div v-i` + Enter gave `v-if=""` and `<button @cl` + Enter gave `@click="go"`. | fixed | With an HTML or Vue language server running, its own suggestions are used instead. |
| P5 colors and semantic kinds | Colors had only a gutter swatch, in style sheets. The bundled Color Preview plugin painted `#hex` anywhere, comments included, with no names. Classes, interfaces, enums and type parameters shared one color. Themes' `semanticTokenColors` were ignored, and token modifiers too, so deprecated code looked like the rest. | `core/src/css.rs` (`colors_for`, `readable_on`), `core/src/theme.rs` (`SemanticRule`, `semantic_style`, captures), `src/app/color_preview.rs` (new), `src/app/lsp_extra.rs`, `src/editor_view.rs` (strikethrough, frames), `src/app/plugins.rs` (retired plugin), settings | Color chips: the color as background, black or white text, frames on near-background colors, see-through colors blended. Found per language: style sheets, pages and SVG, strings in 16 languages, Flutter. Semantic: modifiers read, kinds of type with captures falling back to `type`, `semanticTokenColors` parsed with VS Code's most-specific rule, Dark Modern given Visual Studio 2022's kinds, deprecated struck through. The plugin no longer ships; a bundled copy is removed at start, and one the user installed stays. | 2 css tests (readable text, colors per language), 1 theme test (semantic rule matching and builtin kinds), 1 app test (chips, blending, strings only, frames). App: CSS chips red, yellow, blue, green, white, purple, black and 30% black framed; HTML `style=` tomato, SVG gold and blue; TS strings, comment plain; Rust with rust-analyzer: struct green, enum and trait pale green, const and enum member light blue, deprecated `old` struck through; on a copy of your plugins folder, Color Preview was removed and the rest stayed. | fixed | |
| M7 font previews | Font lists were plain text, with no way to see a font | `core/src/css.rs` (`font_families_at`, `font_names`, `parse_font_families`), `src/app/font_preview.rs` (new), `src/editor_view.rs` (`WordImage`), `src/app/svg_preview.rs`, settings | Each installed family's name, quotes included, is drawn in that font over its own characters (generic ones in what they map to), in its syntax color, at the code font's size and on its baseline, taking its own width with the rest of the row closed up behind it (clicks mapped back to the right char; selections, brackets, chips and squiggles moved with it), except on the cursor's line; missing fonts stay plain. Hovering a list, or the command **CSS: Preview Font Family**, opens a card with a sample per family: installed, missing (drawn in its fallback) or generic, and the one used. Drawing is off the UI thread and cached by theme, font size and scale. Setting `editor.font_previews`. | 2 css tests (lists under the cursor, names and their ranges in CSS, HTML `style=`, TSX), 1 editor test (cells to drawn positions and back). App: names in orange as strings or white as keywords, on the code's baseline, no gaps before the commas; a click on line 1's `}` gave Col 53 and one inside Roboto Col 35; Select All's highlight ended at each closed-up line's end; Segoe UI, Georgia, Times New Roman, Consolas, sans-serif, serif and monospace in their fonts; Roboto and Fira Fantasy plain; the cursor's line plain; hover card on line 3 (Fira Fantasy missing in Consolas, Consolas used, monospace as Courier New) below the Baseline hover; line 1 (Segoe UI used, Roboto in Arial); the command on the `font` shorthand (Georgia used). | fixed | Web fonts loaded by `@font-face` or a link aren't fetched; only installed fonts are drawn. |
| P2 rainbow brackets and structure | Rainbow brackets came only from tree-sitter, so files highlighted with syntect (Ruby, PHP, Kotlin, Swift, Lua, SCSS and the rest) had none, and that's why they seemed missing. Stray brackets looked like any other. A zero-width MISSING bracket could color the next char. Nothing showed the block the cursor is in or a tag's partner. | `core/src/highlight.rs` (`bracket_colors`, `recolor`, `set_rainbow`), `core/src/syntax.rs` (`paint_brackets`), `core/src/theme.rs`, `core/src/auto_tags.rs` (`matching_tag`), `core/src/buffer.rs` (`enclosing_brackets`), `src/editor_view.rs` (`active_guide`, `tag_pair_at`, pair guide), settings | Syntect files: brackets colored by depth from syntect's scope stack, skipping strings, comments and character literals, with the open brackets carried from line to line in its checkpoints. Both paths: a closing bracket with no opener, or the wrong kind, takes `editorBracketHighlight.unexpectedBracket.foreground` (red), and MISSING nodes are skipped. The indent guide of the block holding the cursor takes `editorIndentGuide.activeBackground1`. The bracket pair around the cursor, when it spans lines, gets a guide in its opener's color (VS Code's `editor.guides.bracketPairs: active`). In markup, the tag name under the cursor and its partner are framed. Three new settings: Active indent guide, Bracket pair guide and Matching tags. | 5 new tests: enclosing pairs (nested, after an opener, between pairs, outside), rainbow without a tree (Ruby: nesting across lines, string and comment, stray `)`, off again), stray brackets in Rust, tag matching (nesting, case, void, self-closing, comment, script, XML case, JSX braces). App: Ruby showed gold, orchid and blue by depth across lines, `"(not code"` and `# (comment)` plain, and the stray `)` red; Rust's stray `)` in `oops());` red (pixel ff1212); with the cursor inside the `if` block, its guide was 707070 on lines 4 and 5 and 404040 elsewhere, and on the `if` line itself the block it opens lit; the pair guide down lines 4 and 5 was da70d6, the `if` block's `{`; HTML: the cursor in `</div>` framed both `div`s, in `section` both `section`s, and `<br>` none. | fixed | |
| P2b brackets in strings | Bracket matching, the pair guide, Go to Bracket and Vim's `%` searched the text, so a bracket in a string or comment could pair with a real one. | `core/src/buffer.rs` (`matching_bracket_where`, `enclosing_brackets_where`), `core/src/document.rs` (`matching_bracket`, `enclosing_brackets`, `bracket_ahead`), `core/src/syntax.rs` (`is_literal_at`), `core/src/highlight.rs` (literal ranges per line), `core/src/vim.rs` (bracket hint), `src/editor_view.rs`, `src/app/dispatch.rs` | The searches ask whether each bracket is code: from the syntax tree (strings, character literals and comments, but code interpolated into a string counts), or from syntect's scopes, kept per line with the highlighting. Vim's `%` is given the cursor line's code brackets and partners by the editor. The bracket next to the cursor is cached per text version and position. | 3 tests: a Rust string's `}` and a JS template's interpolation and comment through the tree, with the plain text search shown to be fooled; a Ruby document through syntect (match, no match from inside the string, enclosing pairs, next code bracket); Vim's `%` with the hint. App: Rust with `let s = "}";` matched line 1's `{` to line 6's `}`, the pair guide ran down main's lines 2 to 5, Go to Bracket jumped to line 6, and Vim's `%` went to line 6 and back to line 1; Ruby `foo(")",` matched and guided past the string's `)`. | fixed | |
| B8/P7 Explorer keys and shortcuts | The Explorer had no selection or keyboard focus: rows only marked the open file, and keys went to the editor, so Delete in the Explorer deleted text. Delete, Rename and New were only in the right-click menu; there was no copy, cut or paste of files, no multi-select, and Delete took one item. The keymap knew only "anywhere" and "in the editor". egui sends Ctrl+C, Ctrl+X, Ctrl+V and (on Windows) Shift+Delete as clipboard events, which no binding could match. The Keyboard Shortcuts search box didn't take the keyboard when opened, so typing went into the file. | `src/explorer.rs` (selection, focus, `key_target`, `row_named`, actions), `core/src/file_ops.rs` (new), `core/src/commands.rs` (11 Explorer commands), `src/keymap.rs` (`filesExplorerFocus`, `consume_stroke`), `src/app/files.rs` (`delete_prompt`, `paste_into`), `src/app/dispatch.rs`, `src/app/workbench.rs`, `src/app/keys_ui.rs` | The Explorer takes the keyboard when a row is clicked and keeps it, as VS Code's does. Selection with click, Ctrl+click, Shift+click and Shift+arrows, following the open file; a frame on the keyboard's row, cut rows dimmed. List keys: arrows, Left/Right to close, open, go up or in, Home, End, Page Up/Down, Escape, type-to-jump. Commands bound only while the Explorer has the keyboard: Enter (open, editor takes the keyboard), Space (open, stay), F2, Delete (Recycle Bin), Shift+Delete (for good), Ctrl+C/X/V, Shift+Alt+C, Ctrl+K Ctrl+Shift+Alt+C, Ctrl+A. The prompt covers many items, says when deletion can't be undone, and takes Enter; meanwhile Explorer keys wait. Paste names copies "a copy.txt", "a copy 2.txt", refuses a folder into itself, moves across drives by copying, and moves open files' paths. Clipboard events match their shortcuts in the keymap. The Keyboard Shortcuts editor shows where keys work (Explorer, Editor), "Also used by" says where, and its search box takes the keyboard on open. The open folder can't be deleted, cut or renamed. | 9 new tests: arrows and Left/Right, type-to-jump, the open folder never picked and paste targets, Explorer keys only in the Explorer with clipboard events and Shift+Delete-as-Cut, copy names, copying and moving folders. App, by keystrokes, checked against the files on disk: a click then Down and F2 renamed b.txt to c.txt; Ctrl+C, Ctrl+V made "a copy.txt"; typing "n" jumped to notes.md; Shift+Delete asked "for good", Escape kept it, Enter deleted it; Ctrl+X, Home, Down, Right, Ctrl+V moved "a copy.txt" into src and its open tab followed; Shift+Up selected two rows and Delete asked about both; Left went to src and closed it; Space opened c.txt and "a" then jumped rows; Ctrl+click added a row; a double click and Enter put typed letters in the file (saved Qa.txt, Zb.txt) with no stray newline; Ctrl+Z then Ctrl+Y redid it; Ctrl+H opened Replace; Ctrl+A selected all but the folder; Keyboard Shortcuts listed the 11 Explorer commands marked Explorer. | fixed | |
| B11 recent projects and files | Only up to 10 folders were kept, with no time. Nothing could be removed or cleared. Missing folders were silently hidden, so they could never be removed and stayed in the file. Files weren't remembered at all. | `core/src/recent.rs` (rewritten), `src/app/recent_ui.rs` (new), `src/palette.rs` (removable pickers), `src/app/workbench.rs`, `src/app/files.rs`, `src/app/dispatch.rs`, `core/src/commands.rs` | Projects (20) and files (30) apart, each with when it was last opened, saved as JSON; the old list of folders reads as projects. Open Recent lists projects, then files, each with "Folder · 2 hours ago · not found", then Clear; × or Shift+Delete removes a row and keeps the list open. Welcome page: Recent Projects and Recent Files with × each, More, Clear. File > Open Recent: name and place, not-found ones dimmed, More, Clear. Project menu: recent projects with an × each (missing ones struck through), then Manage Recent. Missing entries stay, struck through and marked; opening one explains why and how to remove it. Commands: File: Clear Recently Opened (asks first) and File: Remove Missing from Recently Opened. | 5 recent tests (order and limits, removing and clearing, missing entries, the old format, times in words). App: an old-format `recent.json` showed both folders on the Welcome page, the missing one struck through; opening the project and a file added the file; Ctrl+R showed Folder, Folder · not found, File · just now and Clear; the missing one said it isn't there and stayed; Shift+Delete removed it with the list still open; Clear asked, then emptied `recent.json`. With a seeded list: the Welcome page's × removed the missing project, the picker's × removed the project, Remove Missing dropped the missing file; File > Open Recent showed names, places and the not-found file dimmed; the project menu showed proj and a struck-through gone, and its × removed gone from `recent.json`. | fixed | |

