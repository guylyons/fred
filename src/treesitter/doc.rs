//! One buffer's syntax tree: parsed incrementally within a time budget, and
//! turned into line styles by the highlight and injection queries.

use super::langs::{self, LangSpec};
use super::scopes;
use crate::buffer::Buffer;
use crate::highlight::{LineStyles, to_style};
use ratatui::style::Style;
use ropey::Rope;
use std::collections::HashMap;
use std::ops::{ControlFlow, Range};
use std::sync::Arc;
use std::time::{Duration, Instant};
use syntect::highlighting::{self as sh, Theme};
use tree_sitter::{
    InputEdit, Node, ParseOptions, ParseState, Parser, Point, QueryCursor, StreamingIterator, Tree,
};

/// Lines painted between two looks at the clock.
const CHUNK: usize = 64;
/// Markdown → fenced HTML → `<script>` JavaScript.
const MAX_DEPTH: usize = 3;

pub struct TsDoc {
    spec: &'static LangSpec,
    parser: Parser,
    /// For injected languages; their trees are not kept.
    inj_parser: Parser,
    tree: Option<Tree>,
    /// The text state (buffer id, version) whose offsets `tree` is in.
    edited_to: Option<(u64, u64)>,
    /// The text state `tree` is a finished parse of.
    parsed: Option<(u64, u64)>,
    /// A cancelled parse of this text state can resume.
    in_progress: Option<(u64, u64)>,
    theme: Arc<Theme>,
    truecolor: bool,
    /// Text no capture colours.
    base: Style,
    /// Per language: what each capture of its highlights query paints.
    styles: HashMap<&'static str, Arc<Vec<Option<Style>>>>,
}

fn point((row, column): (usize, usize)) -> Point {
    Point { row, column }
}

/// The rope from `byte` on, a chunk at a time, and then a final newline:
/// a buffer holds its text without one, and some grammars (Markdown) can't
/// parse a last line that isn't ended. So a tree is one byte longer than
/// the rope, and a node's range can end one past it.
fn read(rope: &Rope, byte: usize) -> &str {
    match byte.cmp(&rope.len_bytes()) {
        std::cmp::Ordering::Less => {
            let (chunk, start, _, _) = rope.chunk_at_byte(byte);
            &chunk[byte - start..]
        }
        std::cmp::Ordering::Equal => "\n",
        std::cmp::Ordering::Greater => "",
    }
}

/// `bytes` of the rope, without the newline `read` adds.
fn slice(rope: &Rope, bytes: Range<usize>) -> ropey::RopeSlice<'_> {
    let len = rope.len_bytes();
    rope.byte_slice(bytes.start.min(len)..bytes.end.min(len))
}

impl TsDoc {
    pub fn new(spec: &'static LangSpec, theme: Arc<Theme>, truecolor: bool) -> TsDoc {
        let mut parser = Parser::new();
        parser
            .set_language(&langs::compiled(spec).language)
            .expect("a bundled grammar loads");
        let base = to_style(sh::Highlighter::new(&theme).get_default(), truecolor);
        TsDoc {
            spec,
            parser,
            inj_parser: Parser::new(),
            tree: None,
            edited_to: None,
            parsed: None,
            in_progress: None,
            theme,
            truecolor,
            base,
            styles: HashMap::new(),
        }
    }

    pub fn name(&self) -> &'static str {
        self.spec.name
    }

    pub fn spec(&self) -> &'static LangSpec {
        self.spec
    }

    /// Bring the tree up to date with `buf`; false if `budget` ran out first.
    fn sync(&mut self, buf: &Buffer, started: Instant, budget: Duration) -> bool {
        let key = (buf.id, buf.version);
        if self.parsed == Some(key) {
            return true;
        }
        if self.in_progress != Some(key) {
            // A cancelled parse of other text must not resume on this text.
            self.parser.reset();
            let edits = match self.edited_to {
                Some((id, version)) if id == buf.id => buf.byte_edits_since(version),
                _ => None,
            };
            match (edits, &mut self.tree) {
                (Some(edits), Some(tree)) => {
                    for e in edits {
                        tree.edit(&InputEdit {
                            start_byte: e.start_byte,
                            old_end_byte: e.old_end_byte,
                            new_end_byte: e.new_end_byte,
                            start_position: point(e.start),
                            old_end_position: point(e.old_end),
                            new_end_position: point(e.new_end),
                        });
                    }
                }
                // Another buffer, or more edits than the log keeps: from scratch.
                _ => self.tree = None,
            }
            self.edited_to = Some(key);
            self.in_progress = Some(key);
        }
        let rope = buf.rope();
        let mut out_of_time = |_: &ParseState| {
            if started.elapsed() >= budget {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        };
        let tree = self.parser.parse_with_options(
            &mut |byte, _| read(rope, byte),
            self.tree.as_ref(),
            Some(ParseOptions::new().progress_callback(&mut out_of_time)),
        );
        let Some(tree) = tree else {
            return false;
        };
        self.tree = Some(tree);
        self.parsed = Some(key);
        self.in_progress = None;
        true
    }

    /// The host language's tree, when it could be brought up to date with
    /// `buf` within `budget` (injected languages are not in it). It is a
    /// parse of the buffer's text plus a final newline: clamp node ranges
    /// to the text's length.
    pub fn tree(&mut self, buf: &Buffer, budget: Duration) -> Option<&Tree> {
        if self.sync(buf, Instant::now(), budget) {
            self.tree.as_ref()
        } else {
            None
        }
    }

    /// Styles for `lines`; a line is None if the tree or its captures weren't ready within
    /// `budget`. The bool is "incomplete: call again soon".
    pub fn styles(
        &mut self,
        buf: &Buffer,
        lines: Range<usize>,
        budget: Duration,
    ) -> (Vec<Option<LineStyles>>, bool) {
        let started = Instant::now();
        let mut out = vec![None; lines.len()];
        let was_ready = self.parsed == Some((buf.id, buf.version));
        if !self.sync(buf, started, budget) {
            return (out, true);
        }
        let rope = buf.rope();
        let end = lines.end.min(rope.len_lines());
        let tree = self.tree.take().expect("sync left a tree");
        let mut incomplete = false;
        let mut from = lines.start;
        while from < end {
            // With the tree already there, the first chunk is always painted:
            // a view gets its colours however small the budget left for it.
            if started.elapsed() >= budget && !(was_ready && from == lines.start) {
                incomplete = true;
                break;
            }
            let to = (from + CHUNK).min(end);
            let mut canvas = Canvas::new(rope, from..to, self.base);
            let range = canvas.range();
            self.layer(self.spec, tree.root_node(), rope, range, 1, &mut canvas);
            for (l, runs) in canvas.runs().into_iter().enumerate() {
                out[from + l - lines.start] = Some(runs);
            }
            from = to;
        }
        self.tree = Some(tree);
        (out, incomplete)
    }

    /// What each capture of `spec`'s highlights query paints: None for
    /// captures that only structure the query.
    fn paints(&mut self, spec: &'static LangSpec) -> Arc<Vec<Option<Style>>> {
        if let Some(p) = self.styles.get(spec.name) {
            return Arc::clone(p);
        }
        let query = &langs::compiled(spec).highlights;
        let themed = scopes::capture_styles(query, &self.theme, self.truecolor);
        let paints = query
            .capture_names()
            .iter()
            .zip(themed)
            .map(|(name, style)| {
                // `@none` and captures the theme leaves plain still cover
                // what is under them (a variable inside a string).
                (*name == "none" || scopes::scope_for(name).is_some())
                    .then(|| style.unwrap_or(self.base))
            })
            .collect();
        let paints = Arc::new(paints);
        self.styles.insert(spec.name, Arc::clone(&paints));
        paints
    }

    /// Paint `spec`'s captures under `root` within `range`, then the
    /// languages injected there, so that injected colours win.
    fn layer(
        &mut self,
        spec: &'static LangSpec,
        root: Node,
        rope: &Rope,
        range: Range<usize>,
        depth: usize,
        canvas: &mut Canvas,
    ) {
        let compiled = langs::compiled(spec);
        let paints = self.paints(spec);
        let text = |n: Node| slice(rope, n.byte_range()).chunks().map(str::as_bytes);
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(range.clone());
        let mut found = vec![];
        let mut captures = cursor.captures(&compiled.highlights, root, text);
        while let Some((m, i)) = captures.next() {
            let c = m.captures()[*i];
            if let Some(style) = paints[c.index as usize] {
                let r = c.node.byte_range();
                found.push((r.start, r.end, m.pattern_index, style));
            }
        }
        // Outer nodes first, so inner ones paint over them; on one node the
        // pattern that should win goes last.
        found.sort_by(|a, b| {
            let by_pattern = if spec.earlier_wins {
                b.2.cmp(&a.2)
            } else {
                a.2.cmp(&b.2)
            };
            a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(by_pattern)
        });
        for (start, end, _, style) in found {
            canvas.paint(start..end, style);
        }

        let Some(query) = compiled.injections.as_ref().filter(|_| depth < MAX_DEPTH) else {
            return;
        };
        // (language, content ranges, the content node's bytes)
        let mut injections: Vec<(&'static LangSpec, Vec<tree_sitter::Range>, Range<usize>)> = vec![];
        let names = query.capture_names();
        let mut matches = cursor.matches(query, root, text);
        while let Some(m) = matches.next() {
            let (mut language, mut content, mut include_children) = (None, None, false);
            for p in query.property_settings(m.pattern_index) {
                match &*p.key {
                    "injection.language" => language = p.value.as_deref().map(str::to_string),
                    "injection.include-children" => include_children = true,
                    // shortcut: `injection.combined` nodes are parsed one by one, so a
                    // construct split across several nodes can colour wrong; parse them
                    // as one document if that shows.
                    _ => {}
                }
            }
            for c in m.captures() {
                match names[c.index as usize] {
                    "injection.language" => {
                        language = Some(slice(rope, c.node.byte_range()).to_string())
                    }
                    "injection.content" => content = Some(c.node),
                    _ => {}
                }
            }
            let (Some(language), Some(node)) = (language, content) else {
                continue;
            };
            // A language Fred doesn't bundle keeps the host's colours.
            let Some(inner) = langs::by_injection_name(language.trim()) else {
                continue;
            };
            let ranges = if include_children {
                vec![node.range()]
            } else {
                without_children(node)
            };
            if !ranges.is_empty() {
                injections.push((inner, ranges, node.byte_range()));
            }
        }
        drop(matches);
        for (inner, ranges, bytes) in injections {
            // shortcut: an injected tree is parsed again on every call and without a
            // deadline, so a very long fenced block costs its whole parse per frame;
            // cache the trees per content node if that shows in typing latency.
            let parsed = self
                .inj_parser
                .set_language(&langs::compiled(inner).language)
                .ok()
                .and_then(|()| self.inj_parser.set_included_ranges(&ranges).ok())
                .and_then(|()| {
                    self.inj_parser.reset();
                    self.inj_parser
                        .parse_with_options(&mut |byte, _| read(rope, byte), None, None)
                });
            if let Some(tree) = parsed {
                let within = bytes.start.max(range.start)..bytes.end.min(range.end);
                self.layer(inner, tree.root_node(), rope, within, depth + 1, canvas);
            }
        }
    }
}

/// `node`'s range minus its named children's (as Neovim does: Markdown's
/// `inline` has its punctuation as unnamed children, and they belong to the
/// injected text).
fn without_children(node: Node) -> Vec<tree_sitter::Range> {
    let mut out = vec![];
    let (mut byte, mut point) = (node.start_byte(), node.start_position());
    let mut walk = node.walk();
    let ends = node
        .named_children(&mut walk)
        .map(|c| c.range())
        .chain([tree_sitter::Range {
            start_byte: node.end_byte(),
            end_byte: node.end_byte(),
            start_point: node.end_position(),
            end_point: node.end_position(),
        }]);
    for child in ends {
        if child.start_byte > byte {
            out.push(tree_sitter::Range {
                start_byte: byte,
                end_byte: child.start_byte,
                start_point: point,
                end_point: child.start_point,
            });
        }
        (byte, point) = (child.end_byte, child.end_point);
    }
    out
}

/// A style per byte of some lines (their `\n` left out).
struct Canvas {
    /// Each line's byte range in the buffer.
    lines: Vec<Range<usize>>,
    cells: Vec<Vec<Style>>,
}

impl Canvas {
    fn new(rope: &Rope, lines: Range<usize>, base: Style) -> Canvas {
        let lines: Vec<_> = lines
            .map(|l| {
                let start = rope.line_to_byte(l);
                let next = rope.line_to_byte(l + 1);
                let newline = next > start && rope.byte(next - 1) == b'\n';
                start..next - usize::from(newline)
            })
            .collect();
        let cells = lines.iter().map(|r| vec![base; r.len()]).collect();
        Canvas { lines, cells }
    }

    fn range(&self) -> Range<usize> {
        self.lines.first().map_or(0, |r| r.start)..self.lines.last().map_or(0, |r| r.end)
    }

    fn paint(&mut self, bytes: Range<usize>, style: Style) {
        let first = self.lines.partition_point(|r| r.end <= bytes.start);
        for (line, cells) in self.lines.iter().zip(&mut self.cells).skip(first) {
            if line.start >= bytes.end {
                break;
            }
            let (from, to) = (bytes.start.max(line.start), bytes.end.min(line.end));
            if from < to {
                cells[from - line.start..to - line.start].fill(style);
            }
        }
    }

    /// Each line's cells as runs of one style.
    fn runs(self) -> Vec<LineStyles> {
        self.cells
            .into_iter()
            .map(|cells| {
                let mut runs: LineStyles = vec![];
                for (i, style) in cells.into_iter().enumerate() {
                    match runs.last_mut() {
                        Some((last, r)) if *last == style => r.end = i + 1,
                        _ => runs.push((style, i..i + 1)),
                    }
                }
                runs
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Edit;
    use ratatui::style::Color;
    use std::path::Path;

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
        assert_eq!((l[0].1.start, l.last().unwrap().1.end), (0, 12));
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
        assert!(d.parsed.is_none(), "the parse was cancelled, not finished");
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

    #[test]
    fn tree_is_returned_only_for_the_current_text() {
        let text = (0..2000).map(|i| format!("fn f{i}() {{}}")).collect::<Vec<_>>().join("\n");
        let mut b = Buffer::from_text(&text);
        let mut d = doc("a.rs");
        assert!(d.tree(&b, Duration::ZERO).is_none(), "no time to parse");
        assert_eq!(d.tree(&b, LONG).unwrap().root_node().end_byte(), text.len() + 1, "the text and a final newline");
        b.apply(Edit { start: 0, end: 0, text: "// é\n".into() });
        let root = d.tree(&b, LONG).unwrap().root_node();
        assert_eq!(root.end_byte(), text.len() + 6 + 1);
        assert_eq!(root.child(0).unwrap().kind(), "line_comment");
        assert!(!root.has_error());
    }

    #[test]
    fn last_line_without_a_newline_is_parsed() {
        let d = &mut doc("a.md");
        let mut b = Buffer::from_text("# Title");
        let heading = d.styles(&b, 0..1, LONG).0[0].clone().unwrap();
        b.apply(Edit { start: 7, end: 7, text: "\n\n```rust\nfn".into() });
        let (st, inc) = d.styles(&b, 0..4, LONG);
        assert!(!inc);
        assert_eq!(st[0].as_ref(), Some(&heading));
        assert_ne!(fg_at(&heading, 2), fg_at(st[2].as_ref().unwrap(), 0), "a heading has its own colour");
        assert_eq!(fg_at(st[3].as_ref().unwrap(), 0), PURPLE, "an unclosed fence at the end is still Rust");
    }

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
}
