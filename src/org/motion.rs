//! Element motion and dragging (org.el): org-forward-paragraph,
//! org-forward-element and friends, org-drag-element-*, org-drag-line-*,
//! org-mark-element, org-transpose-element.
//!
//! Upstream works on buffer positions; these run on an [`EBuf`] of the
//! buffer and a full parse tree. Folded text is Fred's hidden lines.

use super::buf::EBuf;
use super::element::{self, NodeId, Tree, Type};
use super::{Prefix, fold};
use crate::editor::{Editor, Mode};

/// A paragraph or its equivalent (org--paragraph-at-point).
struct Para {
    ty: Option<Type>,
    begin: usize,
    end: usize,
    post_aff: usize,
    cb: Option<usize>,
    ce: Option<usize>,
    parent: Option<NodeId>,
}

fn tree(ed: &Editor, b: &EBuf) -> Tree {
    element::parse_buffer(&b.s, &super::settings(ed))
}

/// Point is in hidden (folded) text.
fn hidden_at(ed: &Editor, b: &EBuf, pos: usize) -> bool {
    let (l, _) = b.line_col(pos.min(b.len()));
    l < ed.line_count() && fold::hidden(ed, l)
}

/// The eol of `pos`'s line is invisible: the next line is hidden.
fn folded_after(ed: &Editor, b: &EBuf, pos: usize) -> bool {
    let (l, _) = b.line_col(pos.min(b.len()));
    l + 1 < ed.line_count() && fold::hidden(ed, l + 1)
}

/// The end of the hidden region containing line `l` (end of its last line).
fn fold_end(ed: &Editor, b: &EBuf, mut l: usize) -> usize {
    while l + 1 < ed.line_count() && fold::hidden(ed, l + 1) {
        l += 1;
    }
    b.eol_at(b.pos_of_line(l))
}

fn fold_start(ed: &Editor, b: &EBuf, mut l: usize) -> usize {
    while l > 0 && fold::hidden(ed, l) {
        l -= 1;
    }
    b.eol_at(b.pos_of_line(l))
}

fn para_at(b: &EBuf, t: &Tree) -> Option<Para> {
    let e = element::at_point(&b.s, t, b.pt)?;
    let ty = t.kind(e);
    let mk = |begin: usize, end: usize, parent: Option<NodeId>| Para {
        ty: None,
        begin,
        end,
        post_aff: begin,
        cb: None,
        ce: None,
        parent,
    };
    let triplet = match ty {
        Type::Table | Type::PropertyDrawer => Some(mk(t.begin(e)?, t.end(e)?, t.parent(e))),
        Type::NodeProperty | Type::TableRow => {
            let p = t.parent(e)?;
            Some(mk(t.begin(p)?, t.end(p)?, t.parent(p)))
        }
        Type::Clock | Type::DiarySexp | Type::Keyword => {
            let re = match ty {
                Type::Clock => crate::org_re!(r"^[ \t]*CLOCK:"),
                Type::DiarySexp => crate::org_re!(r"^%%\("),
                _ => crate::org_re!(r"^[ \t]*#\+\S+?:[ \t]*.*$"),
            };
            let looking = |p: usize| re.is_match(&b.s[p..b.eol_at(p)]);
            let end = if t.post_blank(e) > 0 {
                t.end(e)?
            } else {
                let mut x = EBuf::new(&b.s, b.pt);
                x.forward_line(1);
                while !x.eobp() && looking(x.pt) {
                    x.forward_line(1);
                }
                x.skip_chars_forward(" \t\n", None);
                x.bol()
            };
            let mut x = EBuf::new(&b.s, t.begin(e)?);
            while !x.bobp() && looking(x.pt) {
                x.forward_line(-1);
            }
            let begin = if looking(x.pt) { x.pt } else { x.lbp(2) };
            Some(mk(begin, end, t.parent(e)))
        }
        _ => {
            let mut l = t.lineage_of(e, &[Type::PlainList], true);
            while let Some(x) = l
                && let Some(p) = t.parent(x)
                && matches!(t.kind(p), Type::Item | Type::PlainList)
            {
                l = Some(p);
            }
            l.and_then(|l| {
                let n = t.prop(l, "structure").as_list().map_or(0, <[_]>::len);
                let mut x = EBuf::new(&b.s, t.post_affiliated(l)?);
                x.forward_line(n as i64);
                (Some(x.pt) == t.contents_end(l))
                    .then(|| mk(t.begin(l).unwrap(), t.end(l).unwrap(), t.parent(l)))
            })
        }
    };
    Some(triplet.unwrap_or_else(|| Para {
        ty: Some(ty),
        begin: t.begin(e).unwrap_or(0),
        end: t.end(e).unwrap_or(b.len()),
        post_aff: t.post_affiliated(e).or(t.begin(e)).unwrap_or(0),
        cb: t.contents_begin(e),
        ce: t.contents_end(e),
        parent: t.parent(e),
    }))
}

const BLOCKS: &[Type] = &[
    Type::CommentBlock,
    Type::ExampleBlock,
    Type::ExportBlock,
    Type::SrcBlock,
    Type::VerseBlock,
];

/// org--forward-paragraph-once.
fn forward_once(ed: &Editor, b: &mut EBuf, t: &Tree) {
    if b.line().trim().is_empty() {
        b.skip_chars_forward(" \t\n", None);
    }
    if b.eobp() {
        return;
    }
    if hidden_at(ed, b, b.pt) {
        let (l, _) = b.line_col(b.pt);
        b.goto(fold_end(ed, b, l));
        b.forward_line(1);
        return;
    }
    let Some(e) = para_at(b, t) else {
        b.goto(b.len());
        return;
    };
    if e.ty == Some(Type::PlainList) {
        b.goto(b.pt + 1);
        return forward_once(ed, b, t);
    }
    if folded_after(ed, b, e.post_aff) {
        let (l, _) = b.line_col(e.post_aff);
        b.goto(fold_end(ed, b, l));
        b.forward_line(1);
        return;
    }
    if let Some(cb) = e.cb
        && cb > b.pt
        && e.ty != Some(Type::Paragraph)
    {
        b.goto(cb);
        if !b.bolp() {
            forward_once(ed, b, t);
        } else if b.previous_line_empty() {
            b.forward_line(-1);
        }
        return;
    }
    if e.ty.is_some_and(|ty| BLOCKS.contains(&ty)) {
        let start = EBuf::new(&b.s, e.post_aff).lbp(2);
        if b.pt < start {
            b.goto(start);
        } else {
            let mut x = EBuf::new(&b.s, e.end);
            x.skip_chars_backward(" \t\n", None);
            let cend = x.bol();
            if b.pt >= cend {
                b.goto(e.end);
                b.skip_chars_backward(" \t\n", None);
                b.forward_line(1);
            } else {
                let re = crate::org_re!(r"(?m)^[ \t]*\n");
                match re.find_at(&b.s[..cend], b.pt) {
                    Some(m) => {
                        b.goto(m.end());
                        b.forward_line(-1);
                    }
                    None => b.goto(cend),
                }
            }
        }
        return;
    }
    b.goto(e.end);
    b.skip_chars_backward(" \t\n", None);
    b.forward_line(1);
}

/// org--backward-paragraph-once.
fn backward_once(ed: &Editor, b: &mut EBuf, t: &Tree) {
    if b.bobp() {
        return;
    }
    if b.line().trim().is_empty() {
        let mut x = EBuf::new(&b.s, b.pt);
        x.skip_chars_backward(" \t\n", None);
        if x.bobp() {
            b.goto(0);
            return;
        }
    }
    if hidden_at(ed, b, b.pt - 1) {
        let (l, _) = b.line_col(b.pt - 1);
        b.goto(fold_start(ed, b, l).saturating_sub(1));
        return backward_once(ed, b, t);
    }
    let Some(e) = para_at(b, t) else {
        b.goto(0);
        return;
    };
    let reach = |b: &mut EBuf, p: usize| {
        b.goto(p);
        if b.previous_line_empty() {
            let end = b.lep(0);
            if end == 0 || !hidden_at(ed, b, end - 1) {
                b.forward_line(-1);
            }
        }
    };
    if e.begin == b.pt {
        if b.previous_line_empty() && !hidden_at(ed, b, b.lep(0).saturating_sub(1)) {
            b.forward_line(-1);
        } else if let Some(p) = e.parent
            && t.kind(p) != Type::Section
            && Some(e.begin) == t.contents_begin(p)
        {
            reach(b, t.begin(p).unwrap_or(0));
        } else {
            b.goto(b.pt - 1);
            backward_once(ed, b, t);
        }
        return;
    }
    if e.ty == Some(Type::Paragraph) && !EBuf::new(&b.s, e.begin).bolp() {
        let bol = b.bol_at(e.begin);
        return reach(b, bol);
    }
    if folded_after(ed, b, e.post_aff) {
        return reach(b, e.begin);
    }
    if let Some(ce) = e.ce
        && ce <= b.pt
        && e.ty != Some(Type::Paragraph)
    {
        if matches!(e.ty, Some(Type::FootnoteDefinition | Type::PlainList)) {
            b.skip_chars_backward(" \t\n", None);
            backward_once(ed, b, t);
        } else if ce == b.pt {
            b.goto(b.pt - 1);
            backward_once(ed, b, t);
        } else {
            b.goto(ce);
        }
        return;
    }
    if e.ty.is_some_and(|ty| BLOCKS.contains(&ty)) {
        let start = EBuf::new(&b.s, e.post_aff).lbp(2);
        if b.pt > start {
            let mut x = EBuf::new(&b.s, e.end);
            x.skip_chars_backward(" \t\n", None);
            let cend = x.bol();
            if b.pt > cend {
                b.goto(cend);
            } else {
                b.skip_chars_backward(" \t\n", Some(e.begin));
                let re = crate::org_re!(r"(?m)^[ \t]*\n");
                let found = re
                    .find_iter(&b.s[start..b.pt])
                    .last()
                    .map(|m| start + m.start());
                b.goto(found.unwrap_or(start));
            }
            return;
        }
    }
    reach(b, e.begin);
}

fn paragraphs(ed: &mut Editor, n: i64) {
    let mut b = EBuf::load(ed);
    let t = tree(ed, &b);
    for _ in 0..n.unsigned_abs() {
        if n > 0 {
            if b.eobp() {
                break;
            }
            forward_once(ed, &mut b, &t);
        } else {
            if b.bobp() {
                break;
            }
            backward_once(ed, &mut b, &t);
        }
    }
    b.store(ed);
}

/// org-forward-element.
fn forward_element(ed: &mut Editor) -> Result<(), String> {
    let mut b = EBuf::load(ed);
    if b.eobp() {
        return Err("Cannot move further down".into());
    }
    if b.at_heading() {
        let origin = b.pt;
        b.end_of_subtree(true);
        if !b.at_heading() {
            b.goto(origin);
            return Err("Cannot move further down".into());
        }
    } else {
        let t = tree(ed, &b);
        match element::at_point(&b.s, &t, b.pt) {
            None => ed.set_msg("No element at point"),
            Some(e) => {
                let end = t.end(e).unwrap_or(b.len());
                match t.parent(e) {
                    Some(p) if t.contents_end(p) == Some(end) => b.goto(t.end(p).unwrap_or(end)),
                    _ => b.goto(end),
                }
            }
        }
    }
    b.store(ed);
    Ok(())
}

/// org-backward-element.
fn backward_element(ed: &mut Editor) -> Result<(), String> {
    let mut b = EBuf::load(ed);
    if b.bobp() {
        return Err("Cannot move further up".into());
    }
    if b.at_heading() {
        let origin = ed.cur;
        super::structure::forward_heading_same_level(ed, -1, false);
        if ed.cur.line == origin.line {
            let mut x = EBuf::load(ed);
            if x.up_heading_safe().is_none() {
                ed.cur = origin;
                return Err("Cannot move further up".into());
            }
            x.store(ed);
        }
        return Ok(());
    }
    let t = tree(ed, &b);
    match element::at_point(&b.s, &t, b.pt).and_then(|e| Some((e, t.begin(e)?))) {
        None => ed.set_msg("No element at point"),
        Some((_, beg)) if b.pt != beg => b.goto(beg),
        Some((_, beg)) => {
            b.goto(beg);
            b.skip_chars_backward(" \r\t\n", None);
            if !b.bobp()
                && let Some(mut prev) = element::at_point(&b.s, &t, b.pt)
            {
                b.goto(t.begin(prev).unwrap_or(0));
                while let Some(p) = t.parent(prev)
                    && t.kind(p) != Type::OrgData
                    && t.end(p).is_some_and(|e| e <= beg)
                {
                    prev = p;
                    b.goto(t.begin(p).unwrap_or(0));
                }
            }
        }
    }
    b.store(ed);
    Ok(())
}

/// org-up-element.
fn up_element(ed: &mut Editor) -> Result<(), String> {
    let mut b = EBuf::load(ed);
    if b.at_heading() {
        if b.up_heading_safe().is_none() {
            return Err("No surrounding element".into());
        }
        b.store(ed);
        return Ok(());
    }
    let t = tree(ed, &b);
    let mut parent = element::at_point(&b.s, &t, b.pt).and_then(|e| t.parent(e));
    if let Some(p) = parent
        && t.kind(p) == Type::Section
    {
        parent = t.parent(p);
    }
    match parent.filter(|&p| t.kind(p) != Type::OrgData) {
        Some(p) => b.goto(t.begin(p).unwrap_or(0)),
        None => {
            if !b.back_to_heading() {
                return Err("No surrounding element".into());
            }
        }
    }
    b.store(ed);
    Ok(())
}

/// org-down-element.
fn down_element(ed: &mut Editor) -> Result<(), String> {
    let mut b = EBuf::load(ed);
    let t = tree(ed, &b);
    let e = element::at_point(&b.s, &t, b.pt).ok_or("No inner element")?;
    match t.kind(e) {
        Type::PlainList | Type::Table => {
            b.goto(t.contents_begin(e).unwrap_or(b.pt) + 1);
        }
        k if k.is_greater() => {
            if folded_after(ed, &b, b.pt) {
                let l = ed.cur.line;
                fold::show_subtree(ed, l);
            }
            b.goto(t.contents_begin(e).ok_or("No content for this element")?);
        }
        _ => return Err("No inner element".into()),
    }
    b.store(ed);
    Ok(())
}

/// org-drag-element-backward.
fn drag_backward(ed: &mut Editor) -> Result<(), String> {
    let mut b = EBuf::load(ed);
    let t = tree(ed, &b);
    let elem = element::at_point(&b.s, &t, b.pt).ok_or("No element at point")?;
    if t.kind(elem) == Type::Headline {
        // Keep point where it was relative to the heading.
        let h = fold::back_to_heading(ed, ed.cur.line).ok_or("No element at point")?;
        let (dl, col) = (ed.cur.line - h, ed.cur.byte);
        super::structure::move_subtree(ed, -1)?;
        let nh = fold::back_to_heading(ed, ed.cur.line).unwrap_or(ed.cur.line);
        let l = (nh + dl).min(ed.line_count() - 1);
        ed.set_cursor(l, col);
        return Ok(());
    }
    let beg = t.begin(elem).unwrap_or(0);
    let prev = {
        let mut x = EBuf::new(&b.s, beg);
        x.skip_chars_backward(" \r\t\n", None);
        if x.bobp() {
            None
        } else {
            element::at_point(&b.s, &t, x.pt).map(|mut prev| {
                let mut up = prev;
                while let Some(p) = t.parent(up)
                    && t.kind(p) != Type::OrgData
                    && t.end(p).is_some_and(|e| e <= beg)
                {
                    up = p;
                    prev = p;
                }
                prev
            })
        }
    };
    let prev = match prev {
        Some(p) if !element::nested_p(&t, elem, p) => p,
        _ => return Err("Cannot drag element backward".into()),
    };
    let pos = b.pt;
    let (text, _) = element::swap_a_b(&b.s, &t, prev, elem)?;
    let new = t.begin(prev).unwrap_or(0) + (pos - beg);
    b.s = text;
    b.goto(new);
    b.store(ed);
    Ok(())
}

/// org-drag-element-forward.
fn drag_forward(ed: &mut Editor) -> Result<(), String> {
    let mut b = EBuf::load(ed);
    let t = tree(ed, &b);
    let pos = b.pt;
    let elem = element::at_point(&b.s, &t, b.pt).ok_or("No element at point")?;
    let end = t.end(elem).unwrap_or(b.len());
    if end >= b.len() {
        return Err("Cannot drag element forward".into());
    }
    let next = element::at_point(&b.s, &t, end).ok_or("Cannot drag element forward")?;
    if element::nested_p(&t, elem, next)
        || (t.kind(next) == Type::Headline && t.kind(elem) != Type::Headline)
    {
        return Err("Cannot drag element forward".into());
    }
    let size_next = {
        let mut x = EBuf::new(&b.s, t.end(next).unwrap_or(b.len()));
        x.skip_chars_backward(" \r\t\n", None);
        x.forward_line(1);
        let p = if x.eolp() && !x.bolp() {
            x.pt + 1
        } else {
            x.pt
        };
        p - t.begin(next).unwrap_or(0)
    };
    let size_blank = {
        let mut x = EBuf::new(&b.s, end);
        x.skip_chars_backward(" \r\t\n", None);
        x.forward_line(1);
        end - x.pt
    };
    let (text, _) = element::swap_a_b(&b.s, &t, elem, next)?;
    b.s = text;
    b.goto((pos + size_next + size_blank).min(b.len()));
    b.store(ed);
    Ok(())
}

/// org-drag-line-forward (negative: backward).
fn drag_line(ed: &mut Editor, n: i64) {
    for _ in 0..n.unsigned_abs() {
        let l = ed.cur.line;
        let col = super::buf::column(&ed.buf.line(l)[..ed.cur.byte.min(ed.buf.line(l).len())]);
        let other = if n > 0 { l + 1 } else { l.wrapping_sub(1) };
        if other >= ed.line_count() {
            break;
        }
        let (a, c) = (ed.buf.line(l.min(other)), ed.buf.line(l.max(other)));
        super::splice(ed, l.min(other), 2, &[c.to_string(), a.to_string()]);
        let mut b = EBuf::load(ed);
        b.goto(b.pos_of_line(other));
        b.move_to_column(col);
        b.store(ed);
        ed.set_cursor(other, ed.cur.byte);
    }
}

/// org-mark-element: the element as a Visual-line selection, extended to
/// the next element when repeated.
fn mark_element(ed: &mut Editor) {
    let b = EBuf::load(ed);
    let t = tree(ed, &b);
    if let Mode::VisualLine { anchor } = ed.mode {
        let (lo, hi) = (anchor.min(ed.cur.line), anchor.max(ed.cur.line));
        let at = b.pos_of_line((hi + 1).min(ed.line_count() - 1));
        if let Some(e) = element::at_point(&b.s, &t, at) {
            let end = t.end(e).unwrap_or(b.len());
            let (last, _) = b.line_col(end.saturating_sub(1).max(at));
            ed.mode = Mode::VisualLine { anchor: lo };
            ed.set_cursor(last, 0);
        }
        return;
    }
    let Some(e) = element::at_point(&b.s, &t, b.pt) else {
        return;
    };
    let end = t.end(e).unwrap_or(b.len()).min(b.len());
    let (last, _) = b.line_col(end.saturating_sub(1));
    let (first, col) = b.line_col(t.begin(e).unwrap_or(0));
    ed.mode = Mode::VisualLine {
        anchor: last.max(first),
    };
    ed.set_cursor(first, col);
}

/// org-transpose-element.
fn transpose_element(ed: &mut Editor) -> Result<(), String> {
    let mut b = EBuf::load(ed);
    b.skip_chars_forward(" \r\t\n", None);
    let t = tree(ed, &b);
    let end = element::at_point(&b.s, &t, b.pt)
        .and_then(|e| t.end(e))
        .unwrap_or(b.len());
    b.store(ed);
    drag_backward(ed)?;
    let mut b = EBuf::load(ed);
    b.goto(end.min(b.len()));
    b.store(ed);
    Ok(())
}

pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    let n = if arg.is_none() { 1 } else { arg.value() };
    Some(match name {
        "org-forward-paragraph" => {
            paragraphs(ed, n);
            Ok(())
        }
        "org-backward-paragraph" => {
            paragraphs(ed, -n);
            Ok(())
        }
        "org--forward-paragraph-once" => {
            paragraphs(ed, 1);
            Ok(())
        }
        "org--backward-paragraph-once" => {
            paragraphs(ed, -1);
            Ok(())
        }
        "org-forward-element" => forward_element(ed),
        "org-backward-element" => backward_element(ed),
        "org-up-element" => up_element(ed),
        "org-down-element" => down_element(ed),
        "org-drag-element-backward" => drag_backward(ed),
        "org-drag-element-forward" => drag_forward(ed),
        "org-drag-line-forward" => {
            drag_line(ed, n);
            Ok(())
        }
        "org-drag-line-backward" => {
            drag_line(ed, -n);
            Ok(())
        }
        "org-mark-element" => {
            mark_element(ed);
            Ok(())
        }
        "org-transpose-element" => transpose_element(ed),
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
