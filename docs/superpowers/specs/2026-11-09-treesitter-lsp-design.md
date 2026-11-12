# Tree-sitter highlighting and language servers, out of the box

## Intent

Fred should work well the moment it is installed: better highlighting than
today's line-by-line regex colouring, plus language-server features, with no
setup. Anything Fred has to fetch, it fetches by itself, and the user can see
what is happening (Fred.app shows it in a panel; the terminal build in the
status line).

Chosen approach (user's pick, "A"): **bundle tree-sitter grammars in the
binary; download language servers on first use.**

## Today

- `src/highlight.rs`: syntect with bat's syntaxes and themes via `two-face`,
  compiled in. `Highlighter` caches parser state per line, `invalidate(line)`
  drops states from the first dirty line, `styles(buf, lines, budget)` returns
  `Vec<Option<LineStyles>>` (ratatui `Style` + byte range) and reports
  `incomplete` when the time budget runs out. One `Highlighter` lives in
  `Session::hl` for the live buffer; parked buffers keep theirs
  (`session.rs`). Callers: `ui/render.rs` (TUI and the GUI frame builder),
  `gui/minimap.rs`, grep previews (`one_line`).
- Buffers report only the first dirty line (`buf.take_dirty_from()`).
- No language servers. `src/pick/def.rs` fakes go-to-definition by grepping.
  `src/complete/` completes from buffer words, nearby files and paths.
- GUI ↔ core: JSON lines (`src/gui/proto.rs`, `In`/frame structs; Swift side
  in `macos/Sources/Fred`). No progress or notification message exists; the
  app only uses `NSAlert` for fatal errors.
- `K`, `:ai` and `Space c` run Claude Code. `gd` / `Space d` run the grep
  definition picker.

## Goals

1. **Tree-sitter highlighting** for about 25 bundled languages: Rust,
   TypeScript, TSX, JavaScript, Python, Go, C, C++, Swift, Lua, Ruby, Java,
   Bash, HTML, CSS, JSON, TOML, YAML, Markdown (+ inline), Elixir, PHP,
   SQL, Zig, Make, Dockerfile. Highlight queries (`highlights.scm`) and
   injection queries ship with them, so code fences in Markdown, `<script>`
   and `<style>` in HTML, and HTML inside Markdown are coloured by their own
   grammar.
2. **Syntect stays as the fallback** for every language without a bundled
   grammar (bat knows 200+), and for grep previews.
3. **Every existing theme keeps working.** Tree-sitter capture names map onto
   TextMate scopes (`@keyword` → `keyword`, `@function` →
   `entity.name.function`, `@string` → `string`, `@comment` → `comment`,
   `@type` → `entity.name.type`/`support.type`, …) and are coloured by the
   current syntect theme, including `ansi`.
4. **Incremental reparsing.** Edits are fed to tree-sitter as `InputEdit`s so
   a keystroke reparses only what changed. Keystroke latency must not get
   worse (measure with `FRED_LATENCY=1`).
5. **Language-server client** (stdio JSON-RPC), one server per language per
   project root, never blocking the editor. Phase-1 features:
   - semantic-token highlighting layered over tree-sitter's colours;
   - diagnostics: gutter sign, underline (undercurl in the GUI), the message
     on the cursor's line in the status area, and a picker that lists them
     (`]d` / `[d` jump between them, as in Neovim);
   - go-to-definition: `gd` / `Space d` ask the server when one is attached
     and fall back to today's grep picker otherwise;
   - hover: the server's documentation for the symbol under the cursor in a
     popup (GUI: also on mouse hover after a delay);
   - completion: server items merged into the existing autocomplete,
     ranked ahead of word completions.
6. **Automatic installs.** Opening a file in a language whose server is
   missing starts the download in the background. Servers land in
   `~/.local/share/fred/servers/<name>/<version>/` (XDG data dir on Linux)
   and are pinned to a version.
   - Single-file or archive servers (rust-analyzer, clangd, lua-language-
     server, taplo, marksman) and Node itself come from upstream release
     URLs and are verified against a SHA-256 recorded in Fred's server
     table before anything runs.
   - npm servers (typescript-language-server + typescript, Pyright,
     vscode-langservers-extracted for HTML/CSS/JSON, yaml-language-server,
     bash-language-server) are installed by the private Node's `npm ci`
     from a `package.json` + `package-lock.json` that Fred ships per
     server. npm checks every tarball against the lockfile's integrity
     hash, so the whole dependency tree is verified. Their progress is
     indeterminate.
   - They run on a private Node in `~/.local/share/fred/node/<version>/`,
     so a missing or wrong system Node can't break them.
   - `PATH` only, with a one-time "install X for Y support" note when
     missing: gopls, SourceKit-LSP, zls, ruby-lsp, ElixirLS, jdtls,
     intelephense. Dockerfile, SQL and Make get highlighting only.
7. **Visible progress.**
   - Fred.app: a non-modal panel at the window's top-right, one row per job:
     title ("Installing rust-analyzer 2026-11-03"), a progress bar
     (determinate when the size is known), and Cancel. On failure, the row
     turns into an error with the reason, Retry, and Show Details (the log).
     Finished rows fade after a few seconds. The panel never takes keyboard
     focus.
   - Terminal build: the status line shows the newest job and its percentage;
     failures appear as an error message.
   - `:Lsp info` (B9-style; no `:LspInfo` alias) lists each server: state,
     version, path, root, and the last lines of its log.
8. **Settings** in `config.toml` (`[lsp]`): `enabled = true`,
   `auto_install = true`, and per-language overrides (`command`, `args`,
   `enabled = false`).

## Non-goals (this round)

Rename, code actions/quick fixes, formatting, signature help, inlay hints,
workspace symbols, call hierarchy, debugging (DAP), loading extra grammars
at run time, Windows.

## Anti-goals

- Highlighting that needs the network, or waits on a download.
- Anything that blocks typing: parsing, server start-up, downloads.
- A modal dialog or focus change while the user is typing.
- Running a downloaded file without verifying its checksum.
- Changing what `K`, `Space c`, `:ai` or any existing key does.
- Breaking a theme the user already picked.

## Constraints

- Platforms: macOS arm64 and x86_64, Linux x86_64 and arm64.
- Binary size: about 25–35 MB with grammars (it is 11.6 MB today).
- Language servers needing the language's own toolchain (gopls needs Go;
  SourceKit-LSP ships with Xcode) are used from `PATH` only. When missing,
  Fred says once, in the job panel, what to install.
- Keep a single source of truth for the server table (name, version, per
  platform URL + SHA-256, archive type, binary path, or npm package +
  version), compiled into Fred.
- Fred's own ex/leader conventions apply (see `docs/fred-roadmap.md`, B1/B2/
  B9). Wire into today's hand-synced leader and ex tables; B1/B2 migrate
  them later like everything else. Add this work to
  `docs/fred-roadmap.md` as its own track, outside the 100-point
  percentage, and keep its queue and log current with each commit.

## Stages

Each stage ships on its own and leaves Fred better than before:

1. **Done (2026-11-12), with two acceptance items open:** keystroke
   latency is not yet measured, and the release binary (with `ghostty`)
   grew from 13.7 MB to 49.4 MB, over the 25–35 MB constraint above. The
   SQL grammar alone is about 12.8 MB, Swift 4.0, C++ 3.3, TypeScript + TSX
   2.8.
   Tree-sitter highlighting (bundled grammars, theme mapping, incremental
   parsing, injections). Acceptance: latency unchanged, every bundled theme
   colours each bundled language (tested:
   `highlight::tests::every_theme_colours_every_bundled_language`).
2. LSP client using servers from `PATH`, with all phase-1 features.
3. Installer, private Node, job panel (GUI) and status-line progress (TUI).

## Decided by the user

- Approach A: bundle grammars, download servers on first use.
- Both tree-sitter and LSP.
- **No trust step.** Servers start in any folder, as in Neovim, Helix and
  Zed, even though some (rust-analyzer's build scripts and proc macros)
  run project code.

## Decisions made by me (correct any of these)

- **A server on `PATH` wins over a Fred-installed one**, so a user's own
  setup is respected; Fred installs only when none is found.
- **Pins move only with Fred releases.** URLs point at upstream releases;
  a pin that 404s or fails its checksum is a failed job with the reason
  and Retry, never a silent fallback to an unverified file.
- **Downloads use an in-process HTTP client** (`ureq` with rustls) rather
  than shelling out to `curl`, to get byte-accurate progress and
  cancellation. Archives are unpacked in-process (`flate2`, `tar`, `zip`).
- **Keys:** `K` stays Claude and `Space k` stays "search this file's
  lines". Language features get the free `Space l` prefix (which-key
  shows it): `Space l h` hover, `Space l d` diagnostics picker,
  `Space l i` server info, `Space l r` restart the server. `]d` / `[d`
  jump between diagnostics. In the GUI, resting the mouse on a symbol
  shows hover too.
  `]d` / `[d` work in file buffers; special buffers keep their own `[`/`]`.
- **Hover popup** is drawn in the cell grid like Fred's other popups, the
  same in the terminal and the app (Markdown shown as styled text). Mouse
  hover in the app needs a new hover `In` event from Swift.
- **Semantic tokens** only override tree-sitter where the server adds
  information (e.g. a variable vs a parameter vs a type); token types map to
  the same TextMate scopes as captures.

## Left to the builder

- Crate choices for tree-sitter (`tree-sitter`, `tree-sitter-highlight` or a
  hand-rolled query runner) and for each grammar.
- Whether parsing runs on the main thread within the existing time budget or
  on a worker; the latency goal decides.
- How `Buffer` reports edits as `InputEdit`s.
- The exact capture → scope table and the server version pins.
- The protocol message for jobs (`Out` frame field or separate line) and
  the Swift panel's look within the description above.
- Test strategy: fake-server fixtures for the LSP client, a local HTTP
  fixture for the installer, highlight snapshot tests per bundled grammar.
