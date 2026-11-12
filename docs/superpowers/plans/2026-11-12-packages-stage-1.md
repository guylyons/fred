# Packages, stage 1: loader and `pack` commands

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fred installs packages listed in `config.toml` from git, compiles a package's tree-sitter grammar with the system C compiler, and highlights and folds with it. The 25 bundled grammars stay (stage 2 removes them). Twig is the first package.

**Spec:** `docs/superpowers/specs/2026-11-12-packages-and-scripting-design.md` (Goals 1–3, Stage 1).

**Architecture:** A new `src/pack/` module owns manifests, the lock file, syncing and the installed-package list. `src/treesitter/langs.rs` keeps its API (`for_file`, `by_injection_name`, `compiled`, `&'static LangSpec`), but its table becomes `bundled ++ installed`, built once and leaked, and a `LangSpec` can take its `Language` from a shared library and its queries from files. Nothing else in `src/treesitter/`, `src/highlight.rs` or the fold code changes.

**Tech stack:** Rust 2024, `toml` + `serde` (already deps), `libloading` (new), `git` and `cc` run as commands.

## Global constraints

- Opening a file never runs git or a compiler and never touches the network. Only `pack sync` and `pack update` do.
- A broken package disables itself only: its name and error go to the message line once, and to `:pack list`.
- Tests need no network: git sources are local repositories in a temp dir, and the grammar is a vendored `parser.c` (JSON, the smallest) under `tests/fixtures/grammar-json/`.
- `fred --no-packages` loads no package.
- Every implementation commit updates `docs/fred-roadmap.md` (Language intelligence track: queue and log).
- Never stage `notes`; add files by name.

## Review focus

1. **Path safety.** A manifest's `dir`, `file` and `load` entries, and a config entry's `path`, must stay inside the package's checkout (`..` and absolute paths are errors). → Task 1 `manifest_paths_cannot_escape`.
2. **A lock that names a commit the remote no longer has.** Sync reports it for that package and leaves the installed copy alone. → Task 2 `missing_commit_keeps_the_installed_copy`.
3. **ABI mismatch.** A library built for a tree-sitter ABI outside the runtime's range is refused with a message, not loaded. → Task 3 `wrong_abi_is_refused`.
4. **A package that shadows a bundled language.** The package wins (so stage 2 can move languages one at a time). → Task 3 `package_overrides_bundled`.
5. **Sync removes only what it installed.** It deletes directories under `packages/` that the lock file named and the config no longer lists, and nothing else. → Task 2 `sync_removes_only_unlisted_locked_packages`.

---

### Task 1: Manifest and config entries

**Files:** create `src/pack/mod.rs`, `src/pack/manifest.rs`; modify `src/lib.rs`, `src/config.rs` (`Config.package: Vec<PackageEntry>`).

**Produces:**
```rust
pub struct Manifest { pub name: String, pub grammar: Option<Grammar>, pub theme: Vec<ThemeFile>, pub scheme: Option<Scheme> }
pub struct Grammar { pub name: String, pub source: String, pub rev: String, pub dir: String, pub status_name: String,
                     pub extensions: Vec<String>, pub file_names: Vec<String>, pub shebangs: Vec<String>,
                     pub injection_names: Vec<String>, pub inherits: Vec<String> }
impl Manifest { pub fn load(dir: &Path) -> Result<Manifest, String>; }
/// config.toml `[[package]]`: exactly one of `git` and `local`.
pub struct PackageEntry { pub git: Option<String>, pub local: Option<PathBuf>, pub path: Option<String> }
```
- [ ] Tests first: a full manifest parses; unknown keys are errors (`deny_unknown_fields`); `manifest_paths_cannot_escape`; an entry with both or neither of `git`/`local` is a config error naming the entry.
- [ ] Implement; `cargo test --lib pack::manifest config::`.
- [ ] Commit `feat(pack): package manifests and config entries`.

### Task 2: Lock file and sync

**Files:** create `src/pack/sync.rs`, `tests/fixtures/grammar-json/` (`parser.c`, `tree_sitter/parser.h`, from tree-sitter-json at the crate's version, source and licence in a README beside them).

**Produces:**
```rust
pub struct Lock(pub Vec<Locked>);            // ~/.config/fred/packages.lock, TOML
pub struct Locked { pub name: String, pub git: Option<String>, pub path: Option<String>, pub commit: Option<String> }
pub struct Report { pub name: String, pub result: Result<String /* what was done */, String> }
/// Bring `root` (…/fred/packages) in line with `entries`; `update` moves locks to the remote head.
pub fn sync(entries: &[PackageEntry], lock: &mut Lock, root: &Path, update: Option<&str>) -> Vec<Report>;
```
Rules: clone to `root/.repos/<hash of url>`, check out the locked commit into `root/<name>` (a `git worktree`-free copy: `git archive | tar`, or `git checkout-index --prefix`); a `local` entry is symlinked. For a grammar: fetch `source` at `rev` the same way into `root/<name>/.grammar/`, then `cc -shared -fPIC -O2 -I <dir> parser.c [scanner.c] -o grammar.{dylib,so}`; skip when `grammar.*` is newer than the manifest and the rev is unchanged. `$CC` overrides `cc`.
- [ ] Tests first, all with local repos: fresh sync installs and locks the head; a second sync does nothing; `update` moves the lock; `missing_commit_keeps_the_installed_copy`; `sync_removes_only_unlisted_locked_packages`; a grammar package's library exists and exports `tree_sitter_json`; a compiler failure is that package's `Err` with the compiler's first error line.
- [ ] Implement; `cargo test --lib pack::sync`.
- [ ] Commit `feat(pack): sync packages from git and compile grammars`.

### Task 3: Grammars from packages

**Files:** modify `src/treesitter/langs.rs`, `Cargo.toml` (`libloading = "0.8"`); create `src/pack/installed.rs`.

**Produces:**
```rust
// pack/installed.rs
pub struct Installed { pub manifest: Manifest, pub dir: PathBuf }
/// Every package under `root` with a valid manifest; the others as errors.
pub fn installed(root: &Path) -> (Vec<Installed>, Vec<(String, String)>);
// langs.rs
pub enum Source { Bundled(fn() -> Language), Library { path: &'static Path, symbol: &'static str } }
// LangSpec: `language: Source`; queries are `Queries::Inline(&[&str])` or `Queries::Dir(&'static Path)`.
/// Called once at startup, before the first `for_file`; without it only bundled languages exist.
pub fn add_packages(installed: &[Installed]);
pub fn all() -> &'static [LangSpec];        // replaces direct uses of LANGS
```
`compiled` opens the library on first use, checks `Language::abi_version()` against `tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=LANGUAGE_VERSION`, reads `queries/{highlights,injections,folds}.scm` (after those of each `inherits` package), and returns `Result`: a failure is remembered, reported once, and that language falls back to syntect. The library is leaked (never unloaded).
- [ ] Tests first (they build the fixture grammar with Task 2's compile function into a temp dir): a `.json`-like test extension highlights through the package; `package_overrides_bundled`; `wrong_abi_is_refused` (a stub C file whose `tree_sitter_x` returns a `TSLanguage` with `abi_version` 1); a query that doesn't compile disables the package and names it; every existing `treesitter::` and `highlight::` test still passes.
- [ ] Implement; `cargo test --lib`.
- [ ] Commit `feat(highlight): tree-sitter grammars from installed packages`.

### Task 4: `fred pack`, `:pack`, `--no-packages`

**Files:** modify `src/args.rs`, `src/main.rs`, `src/app.rs`, `src/gui/mod.rs` (call `add_packages` before the first `set_file`; show load errors), `src/ex/cmd.rs` + `src/session.rs` (`:pack sync|update [name]|list`, as an `ExEffect` so the session runs it and prints the reports).

- [ ] Tests first: args parse `pack sync`, `pack update x`, `pack list`, `--no-packages`; `:pack list` on a session with one good and one broken package shows both, the broken one with its error; an `e2e` test runs `fred pack sync` against a local repo and then opens a file of that language.
- [ ] Implement; `cargo test`.
- [ ] Commit `feat(pack): fred pack and :pack commands`.

### Task 5: The Twig package and docs

**Files:** create `packages/lang/twig/package.toml`, `packages/lang/twig/queries/{highlights,injections,folds}.scm` (from nvim-treesitter's `runtime/queries/twig/` at the commit `queries/*/folds.scm` already cite, with the same header; the grammar `source` and `rev` are the ones nvim-treesitter's parser table names for twig at that commit); modify `README.md` (a "Packages" section: the config entry, `fred pack sync`, where things are stored), `docs/fred-roadmap.md`, the spec (stage 1 done).

- [ ] Run for real: add the entry to a scratch `config.toml`, `fred pack sync`, open a `.twig` file; check highlighting and `zM`/`zR`. Record the commands and what was seen in the commit message.
- [ ] Every capture name in Twig's highlight query has a scope in `scopes.rs` (extend the table; the existing coverage test must include package queries when the fixture is present).
- [ ] Commit `feat(pack): Twig as the first language package`.
