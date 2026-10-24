# Rust Code

A lightweight VS Code–style editor written in Rust with [egui](https://github.com/emilk/egui).

## Run

```
cargo run --release -- [folder-or-file]
```

## Features

- File explorer (lazy folder tree), tabs with unsaved markers, breadcrumbs
- Custom editor widget on a rope (`ropey`): only visible rows are laid out and drawn, so
  million-line files stay responsive; files over 16 MB / 100k lines open in large-file mode
  (no highlighting or wrap)
- Incremental syntax highlighting (syntect, parser checkpoints every 32 lines), line numbers,
  current-line and matching-bracket highlight
- Multi-cursor: `Ctrl+D` adds the next match, `Ctrl+Alt+Up/Down` adds cursors, `Alt+Click`,
  `Esc` returns to one cursor; per-document undo/redo (`Ctrl+Z`, `Ctrl+Y`) grouped by word
- VS Code editing behaviour: auto-indent on Enter (splits `{|}`), smart Home, `Ctrl+L` line
  select, copy/cut whole line with no selection, `Ctrl+Enter` line below
- Quick Open (`Ctrl+P`), Command Palette (`Ctrl+Shift+P`), Go to Line (`Ctrl+G`)
- Find in file (`Ctrl+F`), Find in files (`Ctrl+Shift+F`)
- Integrated terminal (`` Ctrl+` ``) with history and `cd`
- Save / Save As / Save All
- Hot exit: open folder, tabs and unsaved text are restored on next launch, even after a crash
  (backed up every second to `%APPDATA%\rust-code\session.json`; no "save changes?" prompt on exit)
- Zoom (`Ctrl+=`, `Ctrl+-`, `Ctrl+0`), toggle side bar (`Ctrl+B`)

### VS Code editor actions

Command IDs, labels and default keybindings were taken from VS Code 1.143.1's
shipped `workbench.desktop.main.js` and `nls.messages.json`.

| Command | Keys |
| --- | --- |
| Toggle Line Comment | `Ctrl+/` |
| Move Line Up / Down | `Alt+Up` / `Alt+Down` |
| Copy Line Up / Down | `Shift+Alt+Up` / `Shift+Alt+Down` |
| Delete Line | `Ctrl+Shift+K` |
| Indent / Outdent Line | `Ctrl+]` / `Ctrl+[` (also `Tab` / `Shift+Tab` on a multi-line selection) |
| Add Selection to Next Find Match (single selection) | `Ctrl+D` |
| Go to Bracket | `Ctrl+Shift+\` |
| Toggle Word Wrap | `Alt+Z` |
| Toggle Zen Mode | `Ctrl+K Z` (`Esc` exits) |
| Reopen Closed Editor | `Ctrl+Shift+T` |

Palette only: Join Lines, Duplicate Selection, Sort Lines Ascending/Descending,
Delete Duplicate Lines, Reverse Lines, Transform to Uppercase/Lowercase/Title Case.
