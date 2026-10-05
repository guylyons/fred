//! org-element-at-point and org-element-context over a parse tree.
//!
//! Upstream parses incrementally from the previous headline (or the
//! cache); Fred walks a full parse tree, which yields the same elements
//! with complete lineage.

use super::ast::{NodeId, Tree, Type, Value};

fn blank_prefix(text: &str, pos: usize) -> bool {
    let pos = pos.min(text.len());
    let next = text[pos..].find('\n').map_or(text.len(), |i| pos + i + 1);
    text[..next].bytes().all(|b| b" \r\t\n".contains(&b))
}

/// org-element-at-point: the smallest element containing POS (a list
/// or table at its first item or row), None before the first element.
pub fn at_point(text: &str, t: &Tree, pos: usize) -> Option<NodeId> {
    let el = at_point_1(text, t, pos)?;
    if t.kind(el) == Type::Section && pos < text.len() {
        return at_point(text, t, pos + 1);
    }
    Some(el)
}

fn at_point_1(text: &str, t: &Tree, pos: usize) -> Option<NodeId> {
    if blank_prefix(text, pos) {
        return None;
    }
    let max = text.len();
    let mut parent = t.root;
    'descend: loop {
        for &el in t.contents(parent) {
            let ty = t.kind(el);
            let end = t.end(el).unwrap_or(max);
            if end <= pos && end != max {
                continue;
            }
            if !ty.is_greater() {
                return Some(el);
            }
            if let (Some(cb), Some(ce)) = (t.contents_begin(el), t.contents_end(el))
                && (cb < pos || (cb == pos && !matches!(ty, Type::PlainList | Type::Table)))
                && (ce > pos || (pos == ce && ce == max))
                && !t.contents(el).is_empty()
            {
                parent = el;
                continue 'descend;
            }
            return Some(el);
        }
        return (parent != t.root).then_some(parent);
    }
}

/// org-element-context: the smallest element or object around POS.
pub fn context(text: &str, t: &Tree, pos: usize) -> Option<NodeId> {
    let el = at_point(text, t, pos)?;
    let ty = t.kind(el);
    let bol = text[..pos.min(text.len())].rfind('\n').map_or(0, |i| i + 1);
    let eol = text[pos.min(text.len())..]
        .find('\n')
        .map_or(text.len(), |i| pos + i);
    // A secondary string is contiguous: plain text widths fill the gaps
    // around its objects.
    let span = |ids: &[NodeId]| -> Option<(usize, usize)> {
        let first = ids.iter().position(|&i| t.begin(i).is_some())?;
        let last = ids.iter().rposition(|&i| t.end(i).is_some())?;
        let len = |s: &[NodeId]| {
            s.iter()
                .filter_map(|&i| t.plain_text(i))
                .map(str::len)
                .sum::<usize>()
        };
        let a = t.begin(ids[first])? - len(&ids[..first]);
        let b = t.end(ids[last])? + len(&ids[last + 1..]);
        Some((a, b))
    };
    let (objs, region_end): (Vec<NodeId>, usize) = match t.post_affiliated(el) {
        Some(post) if pos < post => {
            // Parsed affiliated keyword (caption): objects of this line.
            let objs: Vec<NodeId> = caption_objects(t, el)
                .into_iter()
                .filter(|&o| t.begin(o).is_some_and(|b| b >= bol && b <= eol))
                .collect();
            match span(&objs) {
                Some((a, b)) if pos >= a && pos <= b => (objs, b),
                _ => return Some(el),
            }
        }
        _ if ty == Type::Item => {
            let tag = t.secondary(el, "tag").to_vec();
            match span(&tag) {
                Some((a, b))
                    if bol == t.post_affiliated(el).unwrap_or(0) && pos >= a && pos < b =>
                {
                    (tag, b)
                }
                _ => return Some(el),
            }
        }
        _ if matches!(ty, Type::Headline | Type::Inlinetask) => {
            let title = t.secondary(el, "title").to_vec();
            match span(&title) {
                Some((a, b)) if pos >= a && pos <= b => (title, b),
                _ => return Some(el),
            }
        }
        _ if matches!(ty, Type::Paragraph | Type::TableRow | Type::VerseBlock) => {
            match (t.contents_begin(el), t.contents_end(el)) {
                (Some(cb), Some(ce))
                    if pos >= cb && (pos < ce || (pos == ce && ce == text.len())) =>
                {
                    (t.contents(el).to_vec(), ce)
                }
                _ => return Some(el),
            }
        }
        _ => return Some(el),
    };
    let mut parent = el;
    let mut objs = objs;
    let mut region_end = region_end;
    let mut last = None;
    'outer: loop {
        for o in objs.clone() {
            if t.kind(o) == Type::PlainText {
                continue;
            }
            let (Some(b), Some(e)) = (t.begin(o), t.end(o)) else {
                continue;
            };
            if b > pos {
                return Some(last.unwrap_or(parent));
            }
            if e <= pos && e != region_end {
                if e == pos && !matches!(text[..pos].chars().next_back(), Some(' ' | '\t')) {
                    last = Some(o);
                }
                continue;
            }
            if let (Some(cb), Some(ce)) = (t.contents_begin(o), t.contents_end(o))
                && pos >= cb
                && (pos < ce
                    || (pos == ce
                        && (pos == region_end
                            || !matches!(text[..pos].chars().next_back(), Some(' ' | '\t')))))
            {
                parent = o;
                objs = t.contents(o).to_vec();
                region_end = ce;
                continue 'outer;
            }
            return Some(o);
        }
        return Some(last.unwrap_or(parent));
    }
}

/// The objects of an element's parsed affiliated keywords.
fn caption_objects(t: &Tree, el: NodeId) -> Vec<NodeId> {
    fn collect(v: &Value, out: &mut Vec<NodeId>) {
        match v {
            Value::Secondary(s) => out.extend(s),
            Value::List(l) => l.iter().for_each(|x| collect(x, out)),
            Value::Cons(a, b) => {
                collect(a, out);
                collect(b, out);
            }
            _ => {}
        }
    }
    let mut out = vec![];
    collect(t.prop(el, "caption"), &mut out);
    out
}
