# fred scripting: notes (pre-design, not built)

Status: brainstorming paused 2026-10-02. Resume at "Open questions" below.

## Decided
- Own extension system, Emacs-style init file: `~/.config/fred/init.scm`.
- Language: **Steel** (Scheme in Rust, embeddable; Helix plugin work uses it).
- **No** Emacs package / elisp compatibility. Not feasible: packages lean on
  Emacs internals (markers, overlays, text props, buffer-locals, dynamic
  binding, command loop, ~1500 C primitives). Remacs died; rune is years in.
- Treated as architectural: spec → plan → build.

## Where it plugs in (from reading the code)
- Settings: `src/config.rs` (`Config`, from `~/.config/fred/config.toml`).
- Keys: `Editor::handle_key` (`src/editor.rs`) → `src/vim/`.
- Ex commands: `Editor::run_ex` (`src/editor.rs`) → `src/ex/cmd.rs`.
- Messages: `Editor::set_msg` / `set_err`; cursor: `set_cursor`, `Cursor::pos`.
- Gotcha: `:e` replaces the whole `Editor` (`session.switch_to`), so the
  interpreter + user bindings must live outside `Editor` (session/app level)
  or be carried across like `pick::Project`.

## Rough shape (to confirm)
- Load `init.scm` once at startup; errors show in the message line, never
  block opening the file.
- Small Rust-exposed API, grown only as needed. Candidates:
  - settings: `(set! 'numbers #t)`
  - keys: `(bind 'normal "Space w" (lambda () (ex "w")))`
  - commands: `(defcmd "dup" (lambda () ...))`, buffer fns `line`, `insert`, `cursor`
  - hooks: on-open, on-save, per-filetype
- Escape hatch: `(ex "...")` runs any existing `:` command, covers a lot cheaply.

## Open questions (resume here)
1. Day-one scope: settings / key bindings / `:` commands / hooks — which?
2. Does `init.scm` replace `config.toml` or sit alongside it?
3. Startup cost budget (Steel init time vs fred's "get in and out" goal).
4. `--no-init` / safe-mode flag?
