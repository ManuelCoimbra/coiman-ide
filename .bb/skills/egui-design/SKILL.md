---
name: egui-design
description: Visual and interaction design for Coiman's egui desktop IDE. Use when building a new panel, window, sheet, dialog or widget in coiman-ide, or when restyling one, so that it reads as part of a working tool rather than a default egui demo.
---

# egui design for Coiman

Coiman is a code editor. People sit in it for hours, mostly reading text. Good
design here is quiet and dense and stays out of the way. It is also exact: every
color, gap and motion should come from the workbench, not from egui's defaults.
This skill is for designing panels, windows, sheets and widgets in
`crates/coiman-ide`.

## 1. Start from the job, not the widget

Before writing any UI code, write down three lines (in your reply, not in a file):

- **Task:** what someone opens this for, as a verb ("compare two runs", "find the failing test").
- **Frequency:** how often they use it: always visible, several times an hour, or now and then.
- **Primary object:** what the user is looking at (rows, a tree, a graph, a form, a log).

Frequency decides how the panel opens. Things that stay visible belong in an
island or a side panel. Things used several times an hour go in the dock as a
sheet. Rare things go in a tool window placed with `theme::window_spot`. The
primary object decides the layout. Give it the most room and keep controls to
one row along one edge.

## 2. Use the workbench's tokens, never literals

All color comes from `ui::theme::colors()` (a `Palette<Color32>` built from the
current theme). Read `crates/coiman-ide/src/ui/theme.rs` before styling
anything and reuse what is there:

| Need | Use |
| --- | --- |
| Panel or island frame | `theme::island_frame(fill, inner, theme::island_gaps(..))` |
| Background behind open files | `theme::tab_bar_bg()` |
| Value that moves | `theme::glide(ctx, id, target)` |
| Show and hide | `theme::eased(ctx, id, on)` |
| First position of a tool window | `theme::window_spot(ctx, width)` |
| Layout mode | `theme::islands()`, `theme::dock()` |

Rules:

- No `Color32::from_rgb(..)` in feature code. If the palette lacks a role you
  need, add a named role to `coiman_core::appearance::theme::Palette` with a
  fallback that VS Code theme import can fill. Do not hard-code a color for one
  theme.
- Never assume the theme is dark. Check a design against at least one light
  and one dark built-in theme.
- Corner radius and gaps come from the island constants. A widget inside an
  island uses a smaller radius than the island itself (nested radius = outer
  minus padding), or none.
- Status colors (error, warning, info, success) come from the theme's
  diagnostic roles, so they match the squiggles in the editor.

## 3. Typography in a code tool

- Two faces only: the UI proportional face and the editor's monospace face.
  Use monospace only for things the user could paste into code or a terminal,
  such as paths, identifiers, values and commands. Do not use it as a style
  for labels.
- Build hierarchy from size and weight steps that already exist in the
  `egui::Style` text styles. Don't add a size for a single label.
- Sentence case everywhere. Avoid all caps, letter-spaced section headers and
  a label above every group. Let alignment and spacing do the grouping.
- Truncate long paths in the middle (`crates/…/theme.rs`), keep the file name
  visible, and show the full path on hover.

## 4. Density and layout

- One row of controls, aligned to the same edge as the content it controls.
  Put the action the user takes most often where the pointer already is.
- Rows in lists and grids keep one fixed height so virtualized scrolling
  (`show_rows`) stays exact and the eye can track lines.
- Leave empty space on purpose. Use one gap size between groups and a smaller
  one inside them, and don't add separators where the gaps already separate
  things.
- Panels must stay usable at narrow widths. Decide what collapses first, such
  as labels turning into icons and secondary columns hiding, and test it by
  dragging the splitter.

## 5. Motion

- Motion comes from user input: opening a sheet, expanding a node, a value
  changing. Nothing animates on its own.
- Use `glide` and `eased` only. They honor `theme::set_motion`, so reduced
  motion makes changes instant. Never read the clock and animate by hand.
- Request a repaint only while something is actually moving.

## 6. Words

- Name things the way a developer would say them, and use the same verb on the
  control and in its result. A "Run" button leads to a "Running…" status and
  then "Finished in 2.3 s".
- An empty state says what will appear there and how to get it ("No tests found
  in this crate. Add `#[test]` functions or open another crate.").
- Errors say what failed, the input that caused it, and what to do next. They
  don't apologize. Put the raw error text behind a disclosure, and keep it
  copyable.

## 7. Accessibility and input

- Every action can be reached from the keyboard and appears in the command
  palette (`ui/palette.rs`) with a keymap entry when it is frequent
  (`ui/keymap.rs`).
- Focus is visible in every theme. Check that `selection` and `focus` strokes
  have enough contrast against both island and editor backgrounds.
- Give custom-painted widgets a `Response` and `widget_info`, so AccessKit
  (enabled in the workspace's eframe features) can announce them.
- Hit targets are at least 20 logical px tall, even if the visible glyph is
  smaller.

## 8. Process

1. **Brief:** write the three lines from section 1.
2. **Sketch:** draw an ASCII wireframe of the panel at normal and narrow widths,
   and list which palette roles each region uses.
3. **Challenge:** ask whether this looks like egui's default demo or like every
   other dock panel. If one part does, change it and say why. Pick one element
   to be the panel's signature, such as a sparkline, an inline diff or a
   colored gutter, and keep everything else plain.
4. **Build:** reuse helpers from `ui/theme.rs` and from neighboring panels before
   writing new ones.
5. **Look at it:** run the app (`cargo run -p coiman-ide`). Check the panel in
   the islands and classic layouts, in the dock and as a window, and in a
   light and a dark theme. Save screenshots of the result to `docs/redesign/`
   with the next number prefix and compare them with the existing ones there.
6. **Trim:** remove one decoration, such as a border, an icon or a tint, that
   the panel still reads without.
7. **Verify:** run `scripts/check.ps1` (fmt, clippy, tests) before calling it done.

## What to avoid

- egui's stock gray `Visuals` showing through: an unstyled `Window`, a default
  `ScrollArea` bar or a `CollapsingHeader` with the default triangle next to
  styled siblings.
- Cards with drop shadows for content that is really a list.
- Colors used as decoration. In a code tool, color means something (a
  diagnostic, a git state, a token type), so every hue must carry meaning.
- Emoji used as icons. Use the icons in `ui/icons.rs`.
- Any number of nested frames, each with its own border.
