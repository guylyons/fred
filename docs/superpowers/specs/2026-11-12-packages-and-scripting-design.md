# Packages and scripting

Status: design, not built. Continues `2026-10-02-scripting-notes.md` and
replaces the "grammars are bundled" decision of
`2026-11-09-treesitter-lsp-design.md`.

## Intent

Fred's binary should hold the editor, not every language. Tree-sitter
grammars, their queries, themes and the user's own extensions live outside
the binary as packages, and Fred gets a language (Steel, a Scheme) to write
extensions in.

## Today

- `94a533c` bundles 25 tree-sitter grammars. The release binary went from
  12 MB to 48 MB; the grammars' objects are about 36 MB of that (SQL alone
  13 MB).
- `src/treesitter/langs.rs` is a static table (`LANGS`): each `LangSpec`
  names a grammar crate's `LANGUAGE` and `include_str!`s its highlight,
  injection and fold queries.
- Languages without a grammar already fall back to syntect highlighting
  and indent folds.
- Settings come from `~/.config/fred/config.toml` only. There is no
  scripting.

## Goals

1. **A package is a directory.** `~/.local/share/fred/packages/<name>/`
   with a `package.toml` and any of: one grammar, queries, themes, Scheme
   files. Fred loads every installed package at startup.
2. **Packages come from git.** `config.toml` lists them by URL (or a local
   path); a lock file pins each to a commit. Nothing updates on its own.
3. **Grammars are compiled on this machine.** Installing a grammar package
   fetches the grammar's source at its pinned commit and compiles it with
   the system C compiler into a shared library that Fred loads.
4. **No grammar ships in the binary.** All 25 move to packages. Fred with
   no packages still highlights (syntect) and folds (indent).
5. **Steel extensions.** `~/.config/fred/init.scm` and packages' `.scm`
   files can set options, bind keys, define `:` commands and hook events.

## Non-goals (this round)

- A registry or short package names.
- Prebuilt grammar downloads.
- Loading Neovim's installed parsers.
- Emacs or Neovim plugin compatibility (ruled out in October).
- Language servers (stage 2 of the tree-sitter spec; unchanged, later).

## Constraints

- Opening a file never waits on the network or a compiler. Installing is
  an explicit command; a package that isn't installed is reported once in
  the message line and skipped.
- A broken package (bad manifest, grammar that won't load, query that
  won't compile, Scheme error) disables that package only, with its name
  and the error in the message line. Fred still opens the file.
- A grammar is native code and a script can run anything: Fred installs
  only what `config.toml` lists, at the commit in the lock file.
- Startup with packages stays within Fred's "get in and out" goal: target
  under 10 ms added for 25 grammar packages (libraries and queries load on
  first use of the language, not at startup) and under 20 ms for Steel.
- `fred --no-packages` starts with neither packages nor `init.scm`.
- Platforms as today: macOS arm64/x86_64, Linux x86_64/arm64.

## Design

### Manifest

```toml
# package.toml
name = "lang-rust"

[grammar]
name = "rust"                       # the tree_sitter_<name> symbol
source = "https://github.com/tree-sitter/tree-sitter-rust"
rev = "<commit>"
dir = "src"                         # holds parser.c and an optional scanner.c
status_name = "Rust"                # as syntect names it, for the status line
extensions = ["rs"]
file_names = []
shebangs = []
injection_names = ["rust", "rs"]
# queries/highlights.scm, queries/injections.scm, queries/folds.scm beside it
# inherits = ["ecma"]               # queries of another installed package first

[[theme]]
file = "themes/x.tmTheme"

[scheme]
load = ["init.scm"]
```

### Declaring and installing

```toml
# config.toml
[[package]]
git = "https://github.com/<user>/fred-packages"
path = "lang/rust"          # a package may be a subdirectory of a repo

[[package]]
local = "~/code/my-fred-package"
```

- `fred pack sync` (and `:pack sync`): clone or fetch what's listed,
  check out the locked commit (locking the current head for a new entry),
  compile grammars, remove packages no longer listed.
- `:pack update [name]`: move the lock to the remote's head, then sync.
- `:pack list`: installed packages, their commit, and any load error.
- Lock file: `~/.config/fred/packages.lock` (name, git, path, commit).
- Compile: `cc -shared -fPIC -O2 -I <dir> <dir>/parser.c [<dir>/scanner.c]`
  into `<package>/grammar.{dylib,so}`. Git and `cc` are run as commands;
  Fred links no git library.

### Loading

`LANGS` becomes a table built at startup from the installed packages'
manifests (no library or query is read yet). On the first file of a
language Fred opens the library (`libloading`), takes the `Language`,
checks its ABI version against the tree-sitter runtime, and compiles the
queries. `TsDoc`, `scopes.rs`, `folds.rs` and the fold model don't change.

### Scheme

Steel lives at session level (an `:e` replaces the `Editor`). Day one:

- settings: `(set-option! 'foldlevel 2)` — the options table, as `:set`;
  `config.toml` stays and is read first;
- keys: `(bind-key 'normal "Space w" (lambda () (ex "w")))`;
- commands: `(define-command "dup" (lambda (args) ...))`, with buffer
  functions: line text, line count, cursor, insert, delete, replace,
  current file and file type;
- hooks: `(add-hook 'file-opened f)`, `'file-saved`, `'file-type` (with
  the type);
- `(ex "...")` runs any existing `:` command.

Errors go to the message line with the file and line.

Two rules the example `init.scm` (reviewed 2026-11-12) showed are needed:

- a callback's edits undo as one step: Fred wraps every Scheme callback in
  one undo group;
- bindings can be for one buffer: `(bind-key 'normal "Space c" f 'buffer)`
  binds in the current buffer only (what a `file-type` hook wants), on top
  of `buf_maps`, which already exists for `:map <buffer>`.

## Stages

Each ships on its own:

1. **Done 2026-11-12. Package loader and `pack` commands**, data only: manifests, git,
   lock file, grammar compiling, dynamic loading. The 25 grammars stay
   bundled; one grammar not bundled today (Twig, which the user's Neovim
   has) is the first package and proves the path.
2. **Grammars leave the binary.** The 25 become packages under
   `packages/lang/*` in this repo (queries moved from `queries/` and from
   the crates), the grammar crates leave `Cargo.toml`, and the tests that
   need a grammar build it from a pinned source into the test's temp dir.
   The binary returns to about 13 MB.
3. **Steel.** `init.scm`, packages' `.scm` files, the day-one API above.

## Decided by the user

- Nothing language-specific ships in the binary; grammars are packages.
- A package system comes first, then the extension language.
- Grammars are compiled locally from pinned source.
- Packages come from any git repo, pinned by a lock file.
- Scripting day one: settings, key bindings, `:` commands, hooks.
- Steel, `~/.config/fred/init.scm`, no Emacs compatibility (October).

## Decisions made by me (correct any of these)

- Manifests are TOML, so data-only packages need no interpreter.
- Packages are declared in `config.toml`, not `init.scm`, so stages 1–2
  work before Steel exists and there is one place to look.
- First-party language packages live in this repo under `packages/` and
  are installed by this repo's URL plus a path. No second repo to create.
- `config.toml` stays; `init.scm` adds to it.
- Syntect and its syntaxes stay in the binary as the no-package fallback.
- Fold and highlight queries ship in the package, not fetched from
  nvim-treesitter.

## Left to the builder

- Whether sync compiles grammars in parallel.
- How a test gets a compiled grammar without the network (vendored
  `parser.c` for one small grammar, e.g. JSON, is the likely answer).
- The exact Steel function names.
