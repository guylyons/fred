//! Fold levels from a language's fold query, computed as Neovim's
//! `vim.treesitter.foldexpr()` does (runtime/lua/vim/treesitter/_fold.lua).

use crate::buffer::Buffer;
use crate::fold::Level;
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

/// Every line's level: each `@fold` capture longer than `minlines` lines
/// is a fold, nested at most `nestmax` deep.
pub fn levels(tree: &Tree, query: &Query, buf: &Buffer, minlines: usize, nestmax: usize) -> Vec<Level> {
    let n = buf.len_lines();
    let rope = buf.rope();
    let (mut enter, mut leave) = (vec![0usize; n], vec![0usize; n]);
    let fold = query.capture_index_for_name("fold");
    let text = |node: Node| {
        let len = rope.len_bytes();
        let r = node.byte_range();
        rope.byte_slice(r.start.min(len)..r.end.min(len))
            .chunks()
            .map(str::as_bytes)
    };
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), text);
    let mut prev = None;
    while let Some(m) = matches.next() {
        // A quantified capture (`(import)+ @fold`) folds its first node to its last.
        let mut nodes = m.captures().iter().filter(|c| Some(c.index) == fold).map(|c| c.node);
        let Some(first) = nodes.next() else {
            continue;
        };
        let last = nodes.next_back().unwrap_or(first);
        let start = first.start_position().row;
        let end = last.end_position();
        let mut stop = if end.column == 0 { end.row.saturating_sub(1) } else { end.row }.min(n - 1);
        // nvim-treesitter's `#trim!`: the fold stops before trailing blank lines.
        let trim = query
            .general_predicates(m.pattern_index)
            .iter()
            .any(|p| &*p.operator == "trim!");
        while trim && stop > start && buf.line(stop).trim().is_empty() {
            stop -= 1;
        }
        // The same lines twice (a node and its only child) are one fold.
        if stop >= start && stop - start + 1 > minlines && prev != Some((start, stop)) {
            enter[start] += 1;
            leave[stop] += 1;
            prev = Some((start, stop));
        }
    }
    let (mut level, mut left) = (0usize, 0usize);
    (0..n)
        .map(|l| {
            let mut adjusted = (level + enter[l]).saturating_sub(left);
            let mut start = enter[l] > 0;
            left = leave[l];
            // A line that ends one fold and starts another: the first ends
            // a line early, so the second gets this line.
            if start && left > 0 {
                adjusted = adjusted.saturating_sub(left);
                left = 0;
            }
            level = adjusted;
            if adjusted > nestmax {
                start = false;
            }
            Level {
                n: adjusted.min(nestmax),
                start,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::treesitter::langs;
    use std::path::Path;

    fn of(path: &str, text: &str, minlines: usize, nestmax: usize) -> String {
        let spec = langs::for_file(Some(Path::new(path)), "").unwrap();
        let c = langs::compiled(spec);
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&c.language).unwrap();
        let tree = parser.parse(text, None).unwrap();
        levels(&tree, c.folds.as_ref().unwrap(), &Buffer::from_text(text), minlines, nestmax)
            .iter()
            .map(|l| format!("{}{}", if l.start { ">" } else { "" }, l.n))
            .collect::<Vec<_>>()
            .join(" ")
    }

    // The expected levels are Neovim 0.12.6's `vim.treesitter.foldexpr()` for
    // the same text, with nvim-treesitter's parsers and fold queries.
    const PY: &str = "import os\n\nclass A:\n    def f(self):\n        if x:\n            return 1\n        return 2\n\n    def g(self):\n        pass\n\nx = 1";

    #[test]
    fn python_levels_match_neovim() {
        assert_eq!(of("a.py", PY, 1, 20), "0 0 >1 >2 >3 3 2 1 >2 2 0 0");
    }

    #[test]
    fn minlines_and_nestmax_limit_the_folds() {
        let text = "class A:\n    def f(self):\n        if x:\n            return 1\n        return 2";
        // Only the class has more than 4 lines.
        assert_eq!(of("a.py", text, 4, 20), ">1 1 1 1 1");
        // Deeper than 'foldnestmax' stays at it, and starts no fold.
        assert_eq!(of("a.py", text, 1, 1), ">1 1 1 1 1");
    }

    #[test]
    fn javascript_levels_match_neovim() {
        // Two imports fold as one (a quantified capture).
        let text = "import a from \"a\";\nimport b from \"b\";\n\nfunction f(x) {\n  if (x) {\n    return [\n      1,\n      2,\n    ];\n  }\n}\n\nclass K {\n  m() {\n    return 1;\n  }\n}";
        assert_eq!(of("d.js", text, 1, 20), ">1 1 0 >1 >2 >3 3 3 3 2 1 0 >1 >2 2 2 1");
    }

    #[test]
    fn markdown_levels_match_neovim() {
        // Sections and lists stop before their trailing blank lines (`#trim!`).
        let text = "# T\n\ntext\n\n## S\n\n- a\n  - b\n  - c\n\n```js\nfunction f() {\n  return [\n    1,\n  ];\n}\n```\n\n# U\n\nend";
        assert_eq!(of("c.md", text, 1, 20), ">1 1 1 1 >2 2 >3 3 3 2 >3 3 3 3 3 3 3 0 >1 1 1");
    }

    #[test]
    fn every_fold_query_compiles_and_has_a_fold_capture() {
        for spec in langs::LANGS {
            if let Some(q) = &langs::compiled(spec).folds {
                assert!(q.capture_index_for_name("fold").is_some(), "{}", spec.name);
            }
        }
    }
}
