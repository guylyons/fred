# Tree-sitter Highlighting Implementation Plan (stage 1 of 3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Highlight about 25 bundled languages with tree-sitter (incremental, with injections), coloured by the user's existing syntect theme, and fall back to syntect for every other language.

**Architecture:** `Highlighter` (`src/highlight.rs`) keeps its public API. Inside, it holds either today's syntect state or a `TsDoc` (`src/treesitter/doc.rs`). `TsDoc` owns a parser and tree for one buffer, catches up with the buffer through a new `Buffer::byte_edits_since(version)` log, parses within the caller's time budget (tree-sitter resumes a cancelled parse on the next call), and turns highlight-query captures into `LineStyles`. Captures map to TextMate scopes (`src/treesitter/scopes.rs`), so the current syntect theme colours them.

**Tech Stack:** Rust 2024, `tree-sitter` 0.27, grammar crates from crates.io, syntect 5 + two-face 0.5 (themes, fallback).

**Spec:** `docs/superpowers/specs/2026-11-09-treesitter-lsp-design.md` (Goals 1–4, Stage 1).

## Global Constraints

- Platforms: macOS arm64 and x86_64, Linux x86_64 and arm64. Grammars compile from C via `cc`, which the grammar crates do in their build scripts. Nothing gets downloaded at run time.
- Binary size: about 25–35 MB with grammars (11.6 MB today, `~/.cargo/bin/fred`, release + `ghostty`).
- Highlighting never needs the network and never blocks typing. Keystroke latency must not get worse.
- Every existing theme keeps working, including `ansi`. No public `Highlighter` method changes signature.
- `K`, `Space c`, `:ai` and all existing keys are untouched (no keys in this stage).
- Highlighting turns off for files over 10 MB or lines over 20,000 bytes, as today (`MAX_FILE`, `MAX_LINE`).
- Every implementation commit updates `docs/fred-roadmap.md`: the new "Language intelligence" track's queue and log (Task 6 creates the track).
- Don't commit `ascii-art.txt` or `macos/Sources/Fred/main.swift`. They hold the user's own uncommitted work.

## Review Focus

1. **Multibyte text.** Tree-sitter positions are bytes, but `Edit` uses chars. An edit after `é` must not shift colours. → Task 1 `byte_edits_count_bytes_not_chars`.
2. **An edit during an unfinished parse.** Resuming a cancelled parse after the text changed would parse a mix of old and new text. → Task 4 `edit_during_cancelled_parse_restarts`.
3. **More edits than the log keeps.** After more than 1024 versions (`SPLICES`), the edit log can't replay them, so Fred must reparse from scratch rather than colour stale ranges. → Task 4 `too_many_edits_reparses_fully`.
4. **The buffer is replaced under the same highlighter.** Auto-reload (`app.rs:735` calls only `invalidate(0)`) brings a new `Buffer::id` and resets `version` to 0. → Task 4 `new_buffer_id_reparses_fully`.
5. **An injected language Fred doesn't bundle** (a ```` ```haskell ```` fence in Markdown). The host stays coloured, the fence body stays plain, and nothing panics. → Task 5 `unknown_injection_stays_plain`.

---

### Task 1: Byte-level edit log on `Buffer`

**Files:**
- Modify: `src/buffer.rs` (`Buffer` struct ~76-97, `from_text` ~126-143, `apply` ~167-205, tests module)

**Interfaces:**
- Produces:
  ```rust
  /// One edit in tree-sitter's terms: byte offsets, and (row, byte column) points.
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub struct ByteEdit {
      pub start_byte: usize,
      pub old_end_byte: usize,
      pub new_end_byte: usize,
      pub start: (usize, usize),
      pub old_end: (usize, usize),
      pub new_end: (usize, usize),
  }
  impl Buffer {
      /// The edits since `version`, oldest first; None if they're no longer all remembered.
      pub fn byte_edits_since(&self, version: u64) -> Option<impl Iterator<Item = &ByteEdit>>;
  }
  ```
  Kept in a `VecDeque<ByteEdit>` capped at `SPLICES` (1024), pushed in `apply` beside `splices`, with the same eviction rule as `splices_since`.

- [ ] **Step 1: Write the failing tests** in `buffer.rs`'s `mod tests`

  ```rust
  #[test]
  fn byte_edits_count_bytes_not_chars() {
      let mut b = Buffer::from_text("é\nab");
      let v = b.version;
      // Replace "b" (char 3, byte 4) with "xy\nz".
      b.apply(Edit { start: 3, end: 4, text: "xy\nz".into() });
      let e: Vec<_> = b.byte_edits_since(v).unwrap().copied().collect();
      assert_eq!(e, [ByteEdit {
          start_byte: 4, old_end_byte: 5, new_end_byte: 8,
          start: (1, 1), old_end: (1, 2), new_end: (2, 1),
      }]);
  }

  #[test]
  fn byte_edits_forget_past_capacity() {
      let mut b = Buffer::from_text("");
      for _ in 0..1025 { b.apply(Edit { start: 0, end: 0, text: "a".into() }); }
      assert!(b.byte_edits_since(0).is_none());
      assert_eq!(b.byte_edits_since(1).unwrap().count(), 1024);
      assert_eq!(b.byte_edits_since(b.version).unwrap().count(), 0);
  }
  ```

- [ ] **Step 2: Run them to see them fail**

  Run: `cargo test --lib buffer::tests::byte_edits`
  Expected: compile error, `ByteEdit` not found.

- [ ] **Step 3: Implement `ByteEdit` and `byte_edits_since`**, recording in `apply`. Compute the start and old end from the rope before the mutation, and the new end after it. A point's column is `byte - line_to_byte(row)`.

- [ ] **Step 4: Run them to see them pass**

  Run: `cargo test --lib buffer::tests`
  Expected: all pass.

- [ ] **Step 5: Commit**

  ```bash
  git add src/buffer.rs
  git commit -m "feat(buffer): byte-level edit log for incremental parsers"
  ```

---

### Task 2: Grammar dependencies and the language table

**Files:**
- Modify: `Cargo.toml` (`[dependencies]`), `src/lib.rs` (add `pub mod treesitter;`)
- Create: `src/treesitter/mod.rs`, `src/treesitter/langs.rs`
- Create, only where a crate ships no usable query: `queries/<name>/highlights.scm` and `queries/<name>/injections.scm`. Copy them from the grammar's upstream repo at the crate's version, with a first-line comment naming the source URL, commit and licence.

**Interfaces:**
- Produces:
  ```rust
  // src/treesitter/langs.rs
  pub struct LangSpec {
      /// Shown in the status line; equals syntect's name for the same file when syntect knows the language.
      pub name: &'static str,
      /// Names an injection query may use for it ("rust", "rs", "js", "javascript", …), lowercase.
      pub injection_names: &'static [&'static str],
      pub extensions: &'static [&'static str],
      pub file_names: &'static [&'static str],   // "Makefile", "Dockerfile", …
      pub shebangs: &'static [&'static str],     // interpreter names: "python3", "bash", "node", …
      pub language: fn() -> tree_sitter::Language,
      /// Concatenated in order (TypeScript = JavaScript's + TypeScript's; C++ = C's + C++'s).
      pub highlights: &'static [&'static str],
      pub injections: &'static [&'static str],
  }
  pub static LANGS: &[LangSpec];
  pub struct Compiled { pub language: tree_sitter::Language, pub highlights: tree_sitter::Query, pub injections: Option<tree_sitter::Query> }
  /// File name, then extension, then a `#!` first line.
  pub fn for_file(path: Option<&std::path::Path>, first_line: &str) -> Option<&'static LangSpec>;
  pub fn by_injection_name(name: &str) -> Option<&'static LangSpec>; // case-insensitive
  /// Queries compiled once per language (OnceLock per LANGS index).
  pub fn compiled(spec: &'static LangSpec) -> &'static Compiled;
  ```
- Dependency pins (latest as of 2026-11-09): `tree-sitter = "0.27"`, `tree-sitter-language = "0.1"`, `tree-sitter-rust = "0.24"`, `tree-sitter-typescript = "0.23"` (TypeScript + TSX), `tree-sitter-javascript = "0.25"`, `tree-sitter-python = "0.25"`, `tree-sitter-go = "0.25"`, `tree-sitter-c = "0.24"`, `tree-sitter-cpp = "0.23"`, `tree-sitter-swift = "0.7"`, `tree-sitter-lua = "0.5"`, `tree-sitter-ruby = "0.23"`, `tree-sitter-java = "0.23"`, `tree-sitter-bash = "0.25"`, `tree-sitter-html = "0.23"`, `tree-sitter-css = "0.25"`, `tree-sitter-json = "0.24"`, `tree-sitter-toml-ng = "0.7"`, `tree-sitter-yaml = "0.7"`, `tree-sitter-md = "0.5"` (block + inline: two `LangSpec`s, the inline one with no extensions and injection name `markdown_inline`), `tree-sitter-elixir = "0.3"`, `tree-sitter-php = "0.25"`, `tree-sitter-sequel = "0.3"`, `tree-sitter-zig = "1.1"`, `tree-sitter-make = "1.1"`, `tree-sitter-dockerfile-updated = "0.2"`. If a crate's `tree-sitter-language` version can't share one `Language` type with `tree-sitter` 0.27, use the nearest version that can, and note it in the commit message.

- [ ] **Step 1: Write the failing tests** in `langs.rs`

  ```rust
  #[test]
  fn picks_language_by_name_extension_and_shebang() {
      let f = |p: &str, first: &str| for_file(Some(Path::new(p)), first).map(|s| s.name);
      assert_eq!(f("src/x.rs", ""), Some("Rust"));
      assert_eq!(f("a.tsx", ""), for_file(Some(Path::new("b.tsx")), "").map(|s| s.name));
      assert_ne!(f("a.tsx", ""), f("a.ts", ""));
      assert_eq!(f("Makefile", ""), Some("Makefile"));
      assert_eq!(for_file(None, "#!/usr/bin/env python3").map(|s| s.name), Some("Python"));
      assert_eq!(f("x.zzz-unknown", ""), None);
      assert_eq!(by_injection_name("JS").map(|s| s.name), f("a.js", ""));
  }

  #[test]
  fn every_query_compiles_and_names_match_syntect() {
      let ss = two_face::syntax::extra_newlines();
      for spec in LANGS {
          let c = compiled(spec); // panics with the language name on a bad query
          assert!(c.highlights.capture_names().len() > 0, "{}", spec.name);
          if let Some(ext) = spec.extensions.first() {
              if let Some(syn) = ss.find_syntax_by_extension(ext) {
                  assert_eq!(spec.name, syn.name, "status-line name for .{ext}");
              }
          }
      }
  }
  ```

- [ ] **Step 2: Run them to see them fail**

  Run: `cargo test --lib treesitter::langs`
  Expected: compile error, module `treesitter` not found.

- [ ] **Step 3: Add the dependencies and write `LANGS`** with the 25 languages from the spec, plus markdown-inline. Each entry takes the crate's exported `LANGUAGE` / `HIGHLIGHTS_QUERY` / `INJECTIONS_QUERY` constants where they exist, or `include_str!("../../queries/<name>/….scm")` where they don't. `compiled` panics with `"{name}: {err}"` on a query error, so a bad vendored query fails the test, not a user's session.

- [ ] **Step 4: Run them to see them pass**

  Run: `cargo test --lib treesitter::langs`
  Expected: both pass.

- [ ] **Step 5: Commit**

  ```bash
  git add Cargo.toml Cargo.lock src/lib.rs src/treesitter queries
  git commit -m "feat(highlight): bundle tree-sitter grammars and their queries"
  ```

---

### Task 3: Captures coloured by the syntect theme

**Files:**
- Create: `src/treesitter/scopes.rs`
- Modify: `src/highlight.rs`: extract `Highlighter::convert` (lines 248-263) into `pub(crate) fn to_style(st: sh::Style, truecolor: bool) -> Style`, and have `convert`'s callers use it.

**Interfaces:**
- Consumes: `langs::compiled`.
- Produces:
  ```rust
  /// TextMate scope for a capture name, trying shorter prefixes ("function.method.builtin" →
  /// "function.method" → "function"); None for captures that only structure the query
  /// (names starting with "_", "injection.", "local.", or "none").
  pub fn scope_for(capture: &str) -> Option<&'static str>;
  /// One entry per capture index of `query`: its style under `theme`, None when uncoloured.
  pub fn capture_styles(query: &tree_sitter::Query, theme: &syntect::highlighting::Theme, truecolor: bool) -> Vec<Option<ratatui::style::Style>>;
  ```
  Table (extend for every capture name the bundled queries use; the test below enforces coverage): `keyword` → `keyword.control`, `keyword.function` → `storage.type.function`, `keyword.operator` → `keyword.operator`, `operator` → `keyword.operator`, `function` → `entity.name.function`, `function.call` → `variable.function`, `function.method` → `entity.name.function`, `function.macro` → `entity.name.function.macro`, `function.builtin` → `support.function`, `type` → `entity.name.type`, `type.builtin` → `storage.type`, `constructor` → `entity.name.type`, `variable` → `variable.other`, `variable.parameter` → `variable.parameter`, `variable.builtin` → `variable.language`, `property` → `variable.other.member`, `constant` → `constant.other`, `constant.builtin` → `constant.language`, `number` → `constant.numeric`, `boolean` → `constant.language`, `string` → `string.quoted`, `string.escape` → `constant.character.escape`, `string.special` → `string.regexp`, `escape` → `constant.character.escape`, `character` → `constant.character`, `comment` → `comment`, `comment.documentation` → `comment.block.documentation`, `punctuation` → `punctuation`, `punctuation.bracket` → `punctuation.section`, `punctuation.delimiter` → `punctuation.separator`, `punctuation.special` → `punctuation.definition`, `label` → `entity.name.label`, `tag` → `entity.name.tag`, `attribute` → `entity.other.attribute-name`, `namespace`/`module` → `entity.name.namespace`, `embedded` → `meta.embedded`, `text.title`/`markup.heading` → `markup.heading`, `text.emphasis`/`markup.italic` → `markup.italic`, `text.strong`/`markup.bold` → `markup.bold`, `text.uri`/`markup.link.url` → `markup.underline.link`, `text.literal`/`markup.raw` → `markup.raw`. Style lookup: `syntect::highlighting::Highlighter::new(theme).style_for_stack(&[Scope::new(scope)?])`, then `to_style`. Leave a capture uncoloured (None) when the theme's foreground for it equals the theme's default foreground and it has no font style.

- [ ] **Step 1: Write the failing tests** in `scopes.rs`

  ```rust
  #[test]
  fn scope_falls_back_to_shorter_names() {
      assert_eq!(scope_for("function.method.builtin"), scope_for("function.method"));
      assert_eq!(scope_for("keyword.whatever.new"), scope_for("keyword"));
      assert_eq!(scope_for("_hidden"), None);
      assert_eq!(scope_for("injection.content"), None);
  }

  #[test]
  fn material_dark_keyword_is_purple() {
      let theme = crate::highlight::find_theme("material-dark").unwrap(); // make find_theme pub(crate)
      let c = crate::treesitter::langs::compiled(crate::treesitter::langs::for_file(Some(Path::new("x.rs")), "").unwrap());
      let st = capture_styles(&c.highlights, &theme, true);
      let kw = c.highlights.capture_names().iter().position(|n| *n == "keyword").unwrap();
      assert_eq!(st[kw].and_then(|s| s.fg), Some(Color::Rgb(0xc7, 0x92, 0xea)));
  }

  #[test]
  fn every_bundled_capture_name_has_a_scope_or_is_structural() {
      for spec in crate::treesitter::langs::LANGS {
          for name in crate::treesitter::langs::compiled(spec).highlights.capture_names() {
              let structural = name.starts_with('_') || name.starts_with("injection.") || name.starts_with("local.") || *name == "none" || *name == "spell" || *name == "nospell";
              assert!(structural || scope_for(name).is_some(), "{}: @{name}", spec.name);
          }
      }
  }
  ```

- [ ] **Step 2: Run them to see them fail**

  Run: `cargo test --lib treesitter::scopes`
  Expected: compile error, `scope_for` not found.

- [ ] **Step 3: Implement `scope_for`, `capture_styles`, and `to_style`; make `find_theme` `pub(crate)`.**

- [ ] **Step 4: Run them to see them pass**

  Run: `cargo test --lib treesitter::scopes highlight::tests`
  Expected: all pass. The existing highlight tests still pass after the `to_style` extraction.

- [ ] **Step 5: Commit**

  ```bash
  git add src/treesitter/scopes.rs src/highlight.rs
  git commit -m "feat(highlight): colour tree-sitter captures with the syntect theme"
  ```

---

### Task 4: `TsDoc`: budgeted, incremental parsing and line styles

**Files:**
- Create: `src/treesitter/doc.rs`

**Interfaces:**
- Consumes: `Buffer::byte_edits_since`, `Buffer::{id, version, rope}`, `langs::{LangSpec, compiled}`, `scopes::capture_styles`, `highlight::LineStyles`.
- Produces:
  ```rust
  pub struct TsDoc { /* spec, parser, tree: Option<Tree>, parsed: Option<(u64 /*buf id*/, u64 /*version*/)>,
                       in_progress: Option<(u64, u64)>, theme: Arc<Theme>, truecolor: bool,
                       styles: HashMap<&'static str, Vec<Option<Style>>> } */ }
  impl TsDoc {
      pub fn new(spec: &'static LangSpec, theme: Arc<Theme>, truecolor: bool) -> TsDoc;
      pub fn name(&self) -> &'static str;
      /// Styles for `lines`; a line is None if the tree or its captures weren't ready within
      /// `budget`. The bool is "incomplete: call again soon".
      pub fn styles(&mut self, buf: &Buffer, lines: Range<usize>, budget: Duration) -> (Vec<Option<LineStyles>>, bool);
  }
  ```
- Rules the tests pin:
  - **Sync:** if `parsed` is this `(buf.id, buf.version)`, reuse the tree. If it's the same id and `byte_edits_since(parsed.version)` is `Some`, apply each edit to the tree with `Tree::edit`, then reparse with the old tree. Otherwise (new id, or the log is gone), reparse with no old tree.
  - **Budget:** parse through `Parser::parse_with_options`, reading the rope by chunk (`chunk_at_byte`), with a progress callback that cancels once `budget` has passed. A cancelled parse resumes on the next call only if `in_progress` still equals `(buf.id, buf.version)`. Otherwise call `parser.reset()` first.
  - **Painting:** run the highlights query with `QueryCursor::set_byte_range` over the requested lines. Sort the captures by start ascending, then by length descending, then by pattern index descending, and paint them into a per-line array in that order, so inner nodes and earlier patterns win. Merge adjacent equal styles into runs. A capture spanning lines is split at each line end. Line byte ranges exclude the `\n`.
  - Stop querying when `budget` runs out: the lines not yet reached are None, with incomplete = true.

- [ ] **Step 1: Write the failing tests** in `doc.rs`

  ```rust
  const LONG: Duration = Duration::from_secs(5);
  fn doc(path: &str) -> TsDoc {
      let theme = Arc::new(crate::highlight::find_theme("material-dark").unwrap());
      TsDoc::new(langs::for_file(Some(Path::new(path)), "").unwrap(), theme, true)
  }
  const PURPLE: Option<Color> = Some(Color::Rgb(0xc7, 0x92, 0xea));
  fn fg_at(st: &LineStyles, byte: usize) -> Option<Color> {
      st.iter().find(|(_, r)| r.contains(&byte)).and_then(|(s, _)| s.fg)
  }

  #[test]
  fn colours_rust_keywords_and_covers_the_line() {
      let b = Buffer::from_text("fn main() {}");
      let (st, inc) = doc("a.rs").styles(&b, 0..1, LONG);
      let l = st[0].as_ref().unwrap();
      assert!(!inc);
      assert_eq!(fg_at(l, 0), PURPLE);
  }

  #[test]
  fn closing_a_block_comment_recolours_later_lines() {
      let mut d = doc("a.rs");
      let mut b = Buffer::from_text("/*\nlet b = 2;\n*/\nlet c = 3;");
      let before = d.styles(&b, 1..2, LONG).0[0].clone().unwrap();
      // Turn the block comment opener into a line comment.
      b.apply(Edit { start: 0, end: 2, text: "//".into() });
      let after = d.styles(&b, 1..2, LONG).0[0].clone().unwrap();
      assert_ne!(before, after, "line 1 is no longer inside a comment");
      assert_eq!(fg_at(&after, 0), PURPLE, "`let` is a keyword again");
  }

  #[test]
  fn zero_budget_defers_then_completes() {
      let text = (0..2000).map(|i| format!("fn f{i}() {{}}")).collect::<Vec<_>>().join("\n");
      let b = Buffer::from_text(&text);
      let mut d = doc("a.rs");
      let (st, inc) = d.styles(&b, 1990..1991, Duration::ZERO);
      assert!(inc && st[0].is_none());
      let (st, inc) = d.styles(&b, 1990..1991, LONG);
      assert!(!inc && fg_at(st[0].as_ref().unwrap(), 0) == PURPLE);
  }

  #[test]
  fn edit_during_cancelled_parse_restarts() {
      let text = (0..2000).map(|i| format!("fn f{i}() {{}}")).collect::<Vec<_>>().join("\n");
      let mut b = Buffer::from_text(&text);
      let mut d = doc("a.rs");
      assert!(d.styles(&b, 0..1, Duration::ZERO).1);
      b.apply(Edit { start: 0, end: 0, text: "// ".into() });
      let l = d.styles(&b, 0..1, LONG).0[0].clone().unwrap();
      assert_ne!(fg_at(&l, 3), PURPLE, "`fn` on line 0 is inside a comment now");
  }

  #[test]
  fn too_many_edits_reparses_fully() {
      let mut b = Buffer::from_text("x() {}");
      let mut d = doc("a.rs");
      d.styles(&b, 0..1, LONG);
      for _ in 0..1100 { b.apply(Edit { start: 0, end: 0, text: " ".into() }); }
      b.apply(Edit { start: 0, end: 0, text: "fn ".into() });
      let l = d.styles(&b, 0..1, LONG).0[0].clone().unwrap();
      assert_eq!(fg_at(&l, 0), PURPLE);
  }

  #[test]
  fn new_buffer_id_reparses_fully() {
      let mut d = doc("a.rs");
      d.styles(&Buffer::from_text("// fn"), 0..1, LONG);
      let l = d.styles(&Buffer::from_text("fn x() {}"), 0..1, LONG).0[0].clone().unwrap();
      assert_eq!(fg_at(&l, 0), PURPLE);
  }
  ```

- [ ] **Step 2: Run them to see them fail**

  Run: `cargo test --lib treesitter::doc`
  Expected: compile error, `TsDoc` not found.

- [ ] **Step 3: Implement `TsDoc`** by the rules above (no injections yet).

- [ ] **Step 4: Run them to see them pass**

  Run: `cargo test --lib treesitter::doc`
  Expected: all six pass.

- [ ] **Step 5: Commit**

  ```bash
  git add src/treesitter/doc.rs src/treesitter/mod.rs
  git commit -m "feat(highlight): incremental, time-budgeted tree-sitter documents"
  ```

---

### Task 5: Injections

**Files:**
- Modify: `src/treesitter/doc.rs`

**Interfaces:**
- Consumes: `langs::by_injection_name`, `Compiled::injections`.
- Produces: no new public items. `TsDoc::styles` also paints injected languages.
- Rules:
  - Run the host's injection query over the requested byte range. For each match:
    - The language comes from the `@injection.language` capture's text, or from the `injection.language` `#set!` property.
    - The content is the `@injection.content` node, trimmed of its children unless `injection.include-children` is set.
  - Parse each content range with a parser for that language, using `set_included_ranges`. Query it over the same byte range, and paint its captures **after** the host's, so injected colours win. Recurse up to 3 levels deep (Markdown → fenced HTML → `<script>` JS).
  - Treat `injection.combined` the same as single-node injection (limitation noted in the commit).
  - Injected trees are not cached between calls.
  - A language name `by_injection_name` doesn't know is skipped silently.

- [ ] **Step 1: Write the failing tests**

  ```rust
  #[test]
  fn markdown_fence_is_coloured_as_its_language() {
      let b = Buffer::from_text("# T\n\n```rust\nfn main() {}\n```\n");
      let l = doc("a.md").styles(&b, 3..4, LONG).0[0].clone().unwrap();
      assert_eq!(fg_at(&l, 0), PURPLE);
  }

  #[test]
  fn html_script_is_coloured_as_javascript() {
      let b = Buffer::from_text("<script>\nconst x = 1;\n</script>");
      let l = doc("a.html").styles(&b, 1..2, LONG).0[0].clone().unwrap();
      assert_eq!(fg_at(&l, 0), doc("a.js").styles(&Buffer::from_text("const x = 1;"), 0..1, LONG).0[0].as_ref().and_then(|l| fg_at(l, 0)));
      assert!(fg_at(&l, 0).is_some());
  }

  #[test]
  fn unknown_injection_stays_plain() {
      let b = Buffer::from_text("# Title\n\n```haskell\nmain = 1\n```\n");
      let (st, inc) = doc("a.md").styles(&b, 0..4, LONG);
      assert!(!inc);
      assert!(st[0].as_ref().unwrap().iter().any(|(s, _)| s.fg.is_some()), "heading still coloured");
      let l = st[3].as_ref().unwrap();
      assert!(l.windows(2).all(|w| w[0].0.fg == w[1].0.fg), "fence body in one colour: {l:?}");
  }
  ```

- [ ] **Step 2: Run them to see them fail**

  Run: `cargo test --lib treesitter::doc`
  Expected: the three new tests fail (fence and script lines take the host's colours).

- [ ] **Step 3: Implement injections** by the rules above.

- [ ] **Step 4: Run them to see them pass**

  Run: `cargo test --lib treesitter::doc`
  Expected: all nine pass.

- [ ] **Step 5: Commit**

  ```bash
  git add src/treesitter/doc.rs
  git commit -m "feat(highlight): tree-sitter language injections"
  ```

---

### Task 6: Switch `Highlighter` over, verify, document

**Files:**
- Modify: `src/highlight.rs`: `Highlighter` gets `ts: Option<TsDoc>`. `blank` leaves it None. `set_file` sets it when `langs::for_file` matches and the file isn't disabled, and then `syntax = spec.name`. `styles` delegates to `TsDoc` and copies its incomplete flag. `invalidate` and `one_line` stay syntect-only. Add tests.
- Modify: `README.md` (highlighting paragraph near line 661; install size in the intro, the performance table, and anything else citing 8.6 MB), `docs/fred-roadmap.md` (new track + log), `docs/superpowers/specs/2026-11-09-treesitter-lsp-design.md` (mark stage 1 done).

**Interfaces:**
- Consumes: `TsDoc`, `langs::for_file`.
- Produces: `Highlighter`'s public API is unchanged. `syntax_name()` returns the `LangSpec` name for bundled languages.

- [ ] **Step 1: Write the failing tests** in `highlight.rs`'s `mod tests`

  ```rust
  #[test]
  fn unbundled_languages_still_use_syntect() {
      let mut h = Highlighter::new("material-dark", true).unwrap();
      let b = Buffer::from_text("main :: IO ()\nmain = pure ()");
      h.set_file(Some(Path::new("x.hs")), &b);
      assert_eq!(h.syntax_name(), "Haskell");
      assert!(h.styles(&b, 0..1, LONG)[0].as_ref().unwrap().iter().any(|(s, _)| s.fg.is_some()));
  }

  #[test]
  fn every_theme_colours_every_bundled_language() {
      // One sample per LangSpec (markdown-inline is reached through Markdown).
      let samples: &[(&str, &str)] = &[("a.rs", "fn main() {}"), ("a.ts", "const x: number = 1;"), ("a.tsx", "const x = <div/>;"),
          ("a.js", "const x = 1;"), ("a.py", "def f(): pass"), ("a.go", "package main"), ("a.c", "int main(void) { return 0; }"),
          ("a.cpp", "class A {};"), ("a.swift", "func f() {}"), ("a.lua", "local x = 1"), ("a.rb", "def f; end"),
          ("A.java", "class A {}"), ("a.sh", "if true; then echo; fi"), ("a.html", "<p class=\"x\">hi</p>"), ("a.css", "a { color: red; }"),
          ("a.json", "{\"a\": 1}"), ("a.toml", "a = 1"), ("a.yaml", "a: 1"), ("a.md", "# Title"), ("a.ex", "defmodule A do end"),
          ("a.php", "<?php function f() {}"), ("a.sql", "SELECT 1;"), ("a.zig", "const x = 1;"), ("Makefile", "all:\n\techo"),
          ("Dockerfile", "FROM alpine")];
      let names: Vec<String> = two_face::theme::EmbeddedLazyThemeSet::theme_names().iter().map(|t| t.as_name().to_string())
          .chain(["material-dark".to_string()]).collect();
      for theme in &names {
          for (path, text) in samples {
              let mut h = Highlighter::new(theme, true).unwrap();
              let b = Buffer::from_text(text);
              h.set_file(Some(Path::new(path)), &b);
              let st = h.styles(&b, 0..1, LONG);
              assert!(st[0].as_ref().is_some_and(|l| l.iter().any(|(s, _)| s.fg.is_some() || !s.add_modifier.is_empty())), "{theme} {path}");
          }
      }
  }
  ```

  Keep the existing tests unchanged. `detects_language`, `highlights_spans`, `multiline_state_is_invalidated_by_edits`, `zero_budget_returns_none_for_unparsed_lines` and `huge_lines_disable_highlighting` must still pass on the tree-sitter path.

  If a theme × language pair fails, fix the scope table (Task 3) or pick a sample with something that theme colours. Never weaken the assertion or skip the theme.

- [ ] **Step 2: Run them to see them fail**

  Run: `cargo test --lib highlight::tests`
  Expected: `every_theme_colours_every_bundled_language` fails for languages bat doesn't colour, and for the TS path not yet wired. `unbundled_languages_still_use_syntect` may already pass. It guards the fallback.

- [ ] **Step 3: Wire `TsDoc` into `Highlighter`** as described under Files.

- [ ] **Step 4: Run the whole suite**

  Run: `cargo test --release --features ghostty`
  Expected: all pass, including `tests/gui.rs` and `ui::tests` (status line shows "Rust" for `main.rs`).

- [ ] **Step 5: Check latency and size**

  - Run `python3 tools/bench.py fred src/session.rs` on `main` (before) and on this branch (after). Expected: median CPU within +10% of before.
  - Run `cargo install --path . --features ghostty`, then `ls -l ~/.cargo/bin/fred`. Expected: ≤ 35 MB.
  - Manual check: open `README.md` and `src/session.rs` in Fred.app with `FRED_LATENCY=1`. Type in both. The overlay's latency should match `main`'s, and the HTML in the README should be coloured.

- [ ] **Step 6: Update the docs**
  - README: tree-sitter for the bundled languages (list them), syntect + bat for the rest, the 10 MB / 20,000-byte limits, the new install size and performance numbers.
  - `docs/fred-roadmap.md`: add a "Language intelligence (outside the 100 points)" section, with stages 1–3 as its queue and stage 1 marked done, and a log line.
  - Spec: mark Stage 1 done.

- [ ] **Step 7: Commit**

  ```bash
  git add src/highlight.rs README.md docs/fred-roadmap.md docs/superpowers/specs/2026-11-09-treesitter-lsp-design.md
  git commit -m "feat(highlight): tree-sitter for bundled languages, syntect for the rest"
  ```
