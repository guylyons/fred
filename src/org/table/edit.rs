//! Navigation and structure editing (org-table.el, "Navigation and
//! Structure Editing" and "Create, Import, and Convert Tables").

use super::doc::{
    Doc, Row, clean_line, is_dataline, is_hline, is_table_line, lazy_re, nw, re, trim, width,
};
use super::formula::{fix_formulas, maybe_eval_formula, maybe_recalculate_line};
use super::{opt_bool, opt_int, opt_string};
use regex::Regex;
use std::sync::LazyLock;

type R = Result<(), String>;

/// org-table-automatic-realign and org-table-may-need-update.
pub fn realign_needed(d: &Doc) -> bool {
    !d.st.no_realign && opt_bool("org-table-automatic-realign", true) && d.st.may_need_update
}

/// re-search-forward "|" up to line `end` (exclusive): point after it.
fn search_bar_fwd(d: &mut Doc, end: usize) -> bool {
    if let Some(i) = d.rest().find('|') {
        d.c += i + 1;
        return true;
    }
    for l in d.l + 1..end.min(d.n()) {
        if let Some(i) = d.lines[l].find('|') {
            d.goto(l, i + 1);
            return true;
        }
    }
    false
}

/// search-backward "|" bounded by the start of line `start`: point on it.
fn search_bar_back(d: &mut Doc, start: usize) -> bool {
    if let Some(i) = d.before().rfind('|') {
        d.c = i;
        return true;
    }
    let mut l = d.l;
    while l > start {
        l -= 1;
        if let Some(i) = d.lines[l].rfind('|') {
            d.goto(l, i);
            return true;
        }
    }
    false
}

/// org-table-next-field.
pub fn next_field(d: &mut Doc) -> R {
    maybe_eval_formula(d)?;
    maybe_recalculate_line(d)?;
    if realign_needed(d) {
        d.align();
    }
    let end = d.table_end();
    if d.at_hline() {
        d.eol();
    }
    let jumps = opt_bool("org-table-tab-jumps-over-hlines", true);
    let ok = (|| {
        if !search_bar_fwd(d, end) {
            return false;
        }
        if d.rest().chars().all(|c| c == ' ' || c == '\t') && !search_bar_fwd(d, end) {
            return false;
        }
        if d.rest().starts_with('-') && jumps {
            for l in d.l + 1..end {
                let line = &d.lines[l];
                let t = line.trim_start_matches([' ', '\t']);
                if t.starts_with('|') && !t[1..].starts_with('-') {
                    let i = line.len() - t.len() + 1;
                    d.goto(l, i);
                    break;
                }
            }
        }
        true
    })();
    if !ok {
        return insert_row(d, true);
    }
    if d.rest().starts_with('-') {
        d.forward_line(-1);
        insert_row(d, true)?;
    } else if d.rest().starts_with(' ') {
        d.forward_char(1);
    }
    Ok(())
}

/// org-table-previous-field.
pub fn previous_field(d: &mut Doc) -> R {
    justify_field_maybe(d, None);
    maybe_recalculate_line(d)?;
    if realign_needed(d) {
        d.align();
    }
    if d.at_hline() {
        d.eol();
    }
    let start = d.table_begin();
    let origin = (d.l, d.c);
    let ok = search_bar_back(d, start) && search_bar_back(d, start) && {
        let mut ok = true;
        while {
            let r = d.rest();
            r.starts_with("|-") || r[1..].chars().all(|c| c == ' ' || c == '\t')
        } {
            if !search_bar_back(d, start) {
                ok = false;
                break;
            }
        }
        ok
    };
    if !ok {
        d.goto(origin.0, origin.1);
        return Err("Cannot move to previous table field".into());
    }
    if d.rest().starts_with('|') {
        d.forward_char(1);
        if d.rest().starts_with(' ') {
            d.forward_char(1);
        }
    }
    Ok(())
}

/// org-table-beginning-of-field.
pub fn beginning_of_field(d: &mut Doc, mut n: i64) -> R {
    let pos = (d.l, d.c);
    while n > 1 {
        n -= 1;
        previous_field(d)?;
    }
    // re-search-backward "|" bounded by the start of the previous line.
    let bound = d.l.saturating_sub(1);
    if !search_bar_back(d, bound) {
        return Err("No more table fields before the current".into());
    }
    d.forward_char(1);
    if d.rest().starts_with(' ') {
        d.forward_char(1);
    }
    if (d.l, d.c) >= pos {
        beginning_of_field(d, 2)?;
    }
    Ok(())
}

/// org-table-end-of-field.
pub fn end_of_field(d: &mut Doc, mut n: i64) -> R {
    let pos = (d.l, d.c);
    while n > 1 {
        n -= 1;
        next_field(d)?;
    }
    if let Some(i) = d.rest().find('|') {
        d.c += i;
        d.skip_back(|c| c == ' ');
        if d.char_before() == Some('|') && d.char_after() == Some(' ') {
            d.forward_char(1);
        }
    }
    if (d.l, d.c) <= pos {
        end_of_field(d, 2)?;
    }
    Ok(())
}

/// Move to the next line, creating one at the end of the buffer.
fn forward_line_or_create(d: &mut Doc) {
    if d.l + 1 >= d.n() {
        d.lines.push(String::new());
    }
    d.goto(d.l + 1, 0);
}

/// org-table-next-row.
pub fn next_row(d: &mut Doc) -> R {
    maybe_eval_formula(d)?;
    maybe_recalculate_line(d)?;
    if realign_needed(d) {
        d.align();
    }
    let col = d.current_column();
    if d.l + 1 >= d.n() {
        insert_row(d, true)?;
        d.goto_column(col, false, false);
        d.skip_back(|c| c != '|');
        if d.rest().starts_with(' ') {
            d.forward_char(1);
        }
        return Ok(());
    }
    forward_line_or_create(d);
    if !d.at_table() || d.at_hline() {
        d.forward_line(-1);
        insert_row(d, true)?;
    }
    d.goto_column(col, false, false);
    d.skip_back(|c| c != '|');
    if d.rest().starts_with(' ') {
        d.forward_char(1);
    }
    Ok(())
}

/// org-table-blank-field: blank the field, return its old contents.
pub fn blank_field(d: &mut Doc) -> Result<Option<String>, String> {
    d.check_inside_data_field(false, false)?;
    if d.interactive && d.region.is_some() {
        let ((l1, c1), (l2, c2)) = d.region.unwrap();
        let clip = d.st.take_clip();
        copy_region(d, (l1, c1), (l2, c2), true)?;
        d.st.put_clip(clip);
        return Ok(None);
    }
    d.skip_back(|c| c != '|');
    d.backward_char(1);
    let rest = d.rest();
    let end = rest[1..].find('|').map_or(rest.len(), |i| i + 1);
    if end > 1 {
        let pos = d.c;
        let m = rest[..end].to_owned();
        let len = width(&m);
        d.replace_in_line(pos..pos + end, &format!("|{}", " ".repeat(len - 1)));
        d.goto(d.l, pos + 2);
        return Ok(Some(m[1..].to_owned()));
    }
    Ok(None)
}

lazy_re!(FIELD_SPEC, r"^@([1-9][0-9]*)\$([1-9][0-9]*)$");

/// org-table-find-dataline: on a rule, move to the next data row.
pub fn find_dataline(d: &mut Doc) -> R {
    if !d.at_hline() {
        return Ok(());
    }
    let col = d.current_col();
    let end = d.table_end();
    d.forward_line(1);
    while d.l < end && d.at_hline() {
        if d.forward_line(1) != 0 {
            break;
        }
    }
    if d.l >= end || !is_table_line(d.line()) {
        return Err("Cannot find data row for column operation".into());
    }
    d.move_to_column(col);
    Ok(())
}

fn need_align_line(d: &Doc) -> bool {
    !d.line().trim_end_matches([' ', '\t']).ends_with('|')
}

fn fix_confirm() -> bool {
    // org-table-fix-formulas-confirm: a prompt cannot interrupt a command
    // here, so the formulas are always fixed (the default, nil).
    true
}

/// org-table-insert-column.
pub fn insert_column(d: &mut Doc) -> R {
    if !d.at_table() {
        return Err("Not at a table".into());
    }
    if need_align_line(d) {
        d.align();
    }
    find_dataline(d)?;
    let col = d.current_column().max(1);
    let (beg, end) = (d.table_begin(), d.table_end());
    let shrunk = d.shrunk();
    let (l0, c0) = (d.l, d.c);
    for l in beg..end {
        d.goto(l, 0);
        if !d.at_hline() {
            d.goto_column(col, true, false);
            d.insert("|");
        }
    }
    d.goto(l0, c0);
    d.goto_column(col, false, false);
    d.align();
    d.set_shrunk(
        shrunk
            .into_iter()
            .map(|c| if c < col { c } else { c + 1 })
            .collect(),
    );
    if fix_confirm() {
        fix_formulas(d, '$', &[], Some(col - 1), 1, None)?;
    }
    Ok(())
}

/// org-table-line-to-dline.
pub fn line_to_dline(dlines: &[usize], line: usize, above: bool) -> Option<usize> {
    // dlines[0] is unused (Emacs vector with nil first).
    if dlines.len() < 2 {
        return None;
    }
    let (min, max) = (1, dlines.len() - 1);
    if dlines[min] > line || dlines[max] < line {
        return None;
    }
    if let Some(i) = (1..dlines.len()).find(|&i| dlines[i] == line) {
        return Some(i);
    }
    let below = (1..dlines.len()).find(|&i| dlines[i] > line)?;
    Some(if above { below - 1 } else { below })
}

/// org-table-analyze's column count: fields of the first data line.
pub fn ncol_of(line: &str) -> usize {
    static SEP: LazyLock<Regex> = LazyLock::new(|| re(r"[ \t]*\|[ \t]*"));
    split_string(line, &SEP).len()
}

/// org-split-string: split at `sep`, ignoring a leading and trailing one.
pub fn split_string(s: &str, sep: &Regex) -> Vec<String> {
    let Some(m) = sep.find(s) else {
        return vec![s.to_owned()];
    };
    let mut out = vec![];
    if m.start() != 0 {
        out.push(s[..m.start()].to_owned());
    }
    let mut i = m.end();
    while i <= s.len() {
        let Some(m) = sep.find_at(s, i) else { break };
        if m.end() == m.start() {
            break;
        }
        out.push(s[i..m.start()].to_owned());
        i = m.end();
    }
    if i < s.len() {
        out.push(s[i..].to_owned());
    }
    out
}

/// org-table--move-cell.
fn move_cell(d: &mut Doc, dr: i64, dc: i64) -> R {
    if !d.check_inside_data_field(false, false)? {
        return Err("No table at point".into());
    }
    let (beg, end) = (d.table_begin(), d.table_end());
    let ncol = (beg..end)
        .find(|&l| is_dataline(&d.lines[l]))
        .map_or(0, |l| ncol_of(&d.lines[l]));
    let nrow = (beg..end).filter(|&l| is_dataline(&d.lines[l])).count();
    let row = d.current_line_index() as i64;
    let col = d.current_column() as i64;
    let (tr, tc) = (row + dr, col + dc);
    if tc < 1 || tr < 1 || tc > ncol as i64 || tr > nrow as i64 {
        return Err("Cannot move cell further".into());
    }
    let (row, col, tr, tc) = (row as usize, col as usize, tr as usize, tc as usize);
    let c1 = d.table_get(Some(row), Some(col)).unwrap_or_default();
    let c2 = d.table_get(Some(tr), Some(tc)).unwrap_or_default();
    d.table_put(Some(row), Some(col), &c2, false);
    d.table_put(Some(tr), Some(tc), &c1, false);
    d.goto_line(tr);
    d.goto_column(tc, false, false);
    d.align();
    Ok(())
}

pub fn move_cell_up(d: &mut Doc) -> R {
    move_cell(d, -1, 0)
}
pub fn move_cell_down(d: &mut Doc) -> R {
    move_cell(d, 1, 0)
}
pub fn move_cell_left(d: &mut Doc) -> R {
    move_cell(d, 0, -1)
}
pub fn move_cell_right(d: &mut Doc) -> R {
    move_cell(d, 0, 1)
}

/// org-table-delete-column.
pub fn delete_column(d: &mut Doc) -> R {
    if !d.at_table() {
        return Err("Not at a table".into());
    }
    find_dataline(d)?;
    if d.rest().chars().all(|c| c == ' ' || c == '\t') {
        // Snap into the last column.
        if let Some(i) = d.before().rfind('|') {
            d.c = i;
        }
    }
    d.check_inside_data_field(false, true)?;
    let col = d.current_column();
    let (beg, end) = (d.table_begin(), d.table_end());
    let shrunk: Vec<usize> = d.shrunk().into_iter().filter(|&c| c != col).collect();
    let line0 = d.l;
    let mut end_l = end;
    let mut l = beg;
    while l < end_l {
        d.goto(l, 0);
        if !d.at_hline() {
            d.goto_column(col, true, true);
            let rest = d.rest();
            if let Some(m) = rest.strip_prefix('|')
                && let Some(i) = m.find('|')
                && i > 0
            {
                let c = d.c;
                d.replace_in_line(c..c + i + 2, "|");
            }
        }
        l += 1;
        end_l = end_l.min(d.n());
    }
    d.goto(line0, 0);
    d.goto_column(col, false, false);
    d.align();
    d.set_shrunk(
        shrunk
            .into_iter()
            .map(|c| if c < col { c } else { c - 1 })
            .collect(),
    );
    if fix_confirm() {
        fix_formulas(
            d,
            '$',
            &[(col.to_string(), "INVALID".into())],
            Some(col),
            -1,
            Some(col),
        )?;
    }
    Ok(())
}

/// org-table-move-column.
pub fn move_column(d: &mut Doc, left: bool) -> R {
    if !d.at_table() {
        return Err("Not at a table".into());
    }
    find_dataline(d)?;
    d.check_inside_data_field(false, true)?;
    let col = d.current_column();
    let col1 = if left { col - 1 } else { col };
    let colpos = if left { col as i64 - 1 } else { col as i64 + 1 };
    if left && col == 1 {
        return Err("Cannot move column further left".into());
    }
    static LAST: LazyLock<Regex> = LazyLock::new(|| re(r"^[^|]*\|[^|]*$"));
    if !left && LAST.is_match(d.rest()) {
        return Err("Cannot move column further right".into());
    }
    let (beg, end) = (d.table_begin(), d.table_end());
    let shrunk = d.shrunk();
    let line0 = d.l;
    static TWO: LazyLock<Regex> = LazyLock::new(|| re(r"^\|([^|]+)\|([^|]+)\|"));
    for l in beg..end {
        d.goto(l, 0);
        if !d.at_hline() {
            d.goto_column(col1, true, true);
            if let Some(m) = TWO.captures(d.rest()) {
                let (a, b) = (m.get(1).unwrap(), m.get(2).unwrap());
                let new = format!("|{}|{}|", b.as_str(), a.as_str());
                let c = d.c;
                let n = m.get(0).unwrap().end();
                d.replace_in_line(c..c + n, &new);
            }
        }
    }
    d.goto(line0, 0);
    d.goto_column(colpos as usize, false, false);
    d.align();
    d.set_shrunk(
        shrunk
            .into_iter()
            .map(|c| {
                if c == col && left {
                    c - 1
                } else if c == col {
                    c + 1
                } else if col == c + 1 && left {
                    c + 1
                } else if col + 1 == c && !left {
                    c - 1
                } else {
                    c
                }
            })
            .collect(),
    );
    if fix_confirm() {
        let (a, b) = (col.to_string(), colpos.to_string());
        fix_formulas(d, '$', &[(a.clone(), b.clone()), (b, a)], None, 0, None)?;
    }
    Ok(())
}

/// org-table-move-row.
pub fn move_row(d: &mut Doc, up: bool) -> R {
    let col = d.current_col();
    let pos = (d.l, d.c);
    let hline1 = d.at_hline();
    let dline1 = d.current_dline() as i64;
    let dline2 = dline1 + if up { -1 } else { 1 };
    if up && d.l == 0 {
        return Err("Cannot move row further".into());
    }
    let target = if up { d.l - 1 } else { d.l + 1 };
    if target >= d.n() || !d.table_line_at(target) {
        d.goto(pos.0, pos.1);
        return Err("Cannot move row further".into());
    }
    let hline2 = is_hline(&d.lines[target]);
    d.lines.swap(pos.0, target);
    d.goto(target, 0);
    d.move_to_column(col);
    if !(hline1 || hline2) && fix_confirm() {
        let (a, b) = (dline1.to_string(), dline2.to_string());
        fix_formulas(d, '@', &[(a.clone(), b.clone()), (b, a)], None, 0, None)?;
    }
    Ok(())
}

lazy_re!(MARK_FIELD, r"^[ \t]*\| *[#*$] *\|");

/// org-table-insert-row: above the current line, or below with `below`.
pub fn insert_row(d: &mut Doc, below: bool) -> R {
    if !d.at_table() {
        return Err("Not at a table".into());
    }
    if need_align_line(d) {
        d.align();
    }
    let line = d.line().to_owned();
    let mut new = clean_line(&line);
    if let Some(m) = MARK_FIELD.find(&line) {
        new.replace_range(..m.end(), m.as_str());
    }
    let at = if below { d.l + 1 } else { d.l };
    d.lines.insert(at, new);
    d.goto(at, 0);
    if let Some(i) = d.rest().find('|') {
        d.c += i + 1;
        if d.rest().starts_with(' ') {
            d.c += 1;
        }
    }
    if d.st.may_need_update || d.st.overlay_coordinates {
        d.align();
    }
    if fix_confirm() {
        let dl = d.current_dline() as i64 - 1;
        fix_formulas(d, '@', &[], Some(dl.max(0) as usize), 1, None)?;
    }
    Ok(())
}

/// org-table-insert-hline: below the current line, or above.
pub fn insert_hline(d: &mut Doc, above: bool) -> R {
    if !d.at_table() {
        return Err("Not at a table".into());
    }
    if need_align_line(d) {
        d.align();
    }
    let mut line = clean_line(d.line());
    let col = d.current_col();
    static BLANK: LazyLock<Regex> = LazyLock::new(|| re(r"\|( +)\|"));
    while let Some(m) = BLANK.captures(&line) {
        let r = m.get(0).unwrap().range();
        let n = m.get(1).unwrap().len();
        line.replace_range(r, &format!("+{}|", "-".repeat(n)));
    }
    if let Some(i) = line.find('+') {
        line.replace_range(i..i + 1, "|");
    }
    let at = if above { d.l } else { d.l + 1 };
    d.insert_lines(at, &[line]);
    d.c = 0;
    d.move_to_column(col);
    if d.st.overlay_coordinates {
        d.align();
    }
    Ok(())
}

/// org-table-hline-and-move.
pub fn hline_and_move(d: &mut Doc, same_column: bool) -> R {
    let col = d.current_column();
    maybe_eval_formula(d)?;
    maybe_recalculate_line(d)?;
    insert_hline(d, false)?;
    d.goto(d.l + 1, 0);
    d.eol();
    if d.l + 1 < d.n() && is_hline(&d.lines[d.l + 1]) {
        d.insert("\n|");
        d.align();
    } else {
        next_field(d)?;
    }
    if same_column {
        d.goto_column(col, false, false);
    }
    Ok(())
}

/// org-table-kill-row.
pub fn kill_row(d: &mut Doc) -> R {
    if !d.at_table() {
        return Err("Not at a table".into());
    }
    let col = d.current_col();
    let dline = (!d.at_hline()).then(|| d.current_dline());
    let l = d.l;
    let killed = d.delete_line(l);
    d.kill = Some((killed + "\n", true));
    d.goto(l.min(d.n() - 1), 0);
    if !d.at_table() && d.l > 0 && d.l == l {
        d.forward_line(-1);
    }
    d.move_to_column(col);
    if let Some(dl) = dline
        && fix_confirm()
    {
        fix_formulas(
            d,
            '@',
            &[(dl.to_string(), "INVALID".into())],
            Some(dl),
            -1,
            Some(dl),
        )?;
    }
    Ok(())
}

// ---- copy down ----

lazy_re!(
    LISP_NUMBER,
    r"^[+-]?([0-9]+\.?|[0-9]*\.[0-9]+)([eE]([+-]?[0-9]+)?)?$"
);
lazy_re!(NUM_PREFIX, r"^[0-9]+");
lazy_re!(NUM_SUFFIX, r"[0-9]+$");
lazy_re!(
    TS3,
    r"[\[<]([0-9]{4})-([0-9]{2})-([0-9]{2})( +[^]>\s0-9+-]+)?"
);

#[derive(Debug, PartialEq)]
enum Inc {
    Number(super::lisp::Num),
    Prefix(super::lisp::Num, String),
    Suffix(super::lisp::Num, String),
    Timestamp(String),
}

fn analyze_inc(field: Option<&str>) -> Option<Inc> {
    use super::lisp::string_to_number as stn;
    let f = field?;
    if !nw(f) {
        return None;
    }
    if LISP_NUMBER.is_match(f) {
        return Some(Inc::Number(stn(f)));
    }
    if let Some(m) = NUM_PREFIX.find(f) {
        return Some(Inc::Prefix(stn(m.as_str()), f[m.end()..].to_owned()));
    }
    if let Some(m) = NUM_SUFFIX.find(f) {
        return Some(Inc::Suffix(stn(m.as_str()), f[..m.start()].to_owned()));
    }
    if TS3.is_match(f) {
        return Some(Inc::Timestamp(f.to_owned()));
    }
    None
}

/// org-table--increment-field.
pub fn increment_field(field: &str, previous: Option<&str>) -> String {
    use super::lisp::{Num, num_to_string};
    let step = match super::super::options::get("org-table-copy-increment") {
        Some(toml::Value::Integer(i)) => i,
        _ => 1,
    };
    let next = |n1: &Num, n2: Option<&Num>| -> String {
        num_to_string(&match n2 {
            Some(n2) => n1.add(&n1.sub(n2)),
            None => n1.add(&Num::I(step)),
        })
    };
    match (analyze_inc(Some(field)), analyze_inc(previous)) {
        (Some(Inc::Number(n1)), Some(Inc::Number(n2))) => next(&n1, Some(&n2)),
        (Some(Inc::Number(n)), _) => next(&n, None),
        (Some(Inc::Prefix(n1, p1)), Some(Inc::Prefix(n2, p2))) => {
            format!("{}{}", next(&n1, (p1 == p2).then_some(&n2)), p1)
        }
        (Some(Inc::Prefix(n, p)), _) => format!("{}{}", next(&n, None), p),
        (Some(Inc::Suffix(n1, p1)), Some(Inc::Suffix(n2, p2))) => {
            format!("{}{}", p1, next(&n1, (p1 == p2).then_some(&n2)))
        }
        (Some(Inc::Suffix(n, p)), _) => format!("{}{}", p, next(&n, None)),
        (Some(Inc::Timestamp(t1)), Some(Inc::Timestamp(t2))) => {
            let delta = ts_days(&t1).zip(ts_days(&t2)).map_or(step, |(a, b)| a - b);
            shift_timestamp(&t1, delta)
        }
        (Some(Inc::Timestamp(t1)), _) => shift_timestamp(&t1, step),
        _ => field.to_owned(),
    }
}

fn ts_days(s: &str) -> Option<i64> {
    let m = TS3.captures(s)?;
    Some(super::days_from_civil(
        m[1].parse().ok()?,
        m[2].parse().ok()?,
        m[3].parse().ok()?,
    ))
}

/// org-timestamp-up-day on the first timestamp of `s`.
pub fn shift_timestamp(s: &str, days: i64) -> String {
    let Some(m) = TS3.captures(s) else {
        return s.to_owned();
    };
    let Some(n) = ts_days(s) else {
        return s.to_owned();
    };
    let (y, mo, da) = super::civil_from_days(n + days);
    let open = &m[0][..1];
    let mut new = format!("{open}{y:04}-{mo:02}-{da:02}");
    if m.get(4).is_some() {
        new.push(' ');
        new.push_str(super::weekday_name(n + days));
    }
    let r = m.get(0).unwrap().range();
    format!("{}{}{}", &s[..r.start], new, &s[r.end..])
}

/// org-table-copy-down.
pub fn copy_down(d: &mut Doc, mut n: i64) -> R {
    d.check_inside_data_field(false, false)?;
    let beg = d.table_begin();
    let column = d.current_column();
    let (l0, c0) = (d.l, d.c);
    let initial = {
        let f = d.get_field(None, None);
        d.goto(l0, c0);
        nw(&f).then(|| trim(&f).to_owned())
    };
    let increment = super::super::options::get("org-table-copy-increment");
    let inc_on = !matches!(increment, Some(toml::Value::Boolean(false)));
    let numeric_inc = matches!(increment, Some(toml::Value::Integer(_)));
    let field = match &initial {
        Some(f) => f.clone(),
        None => {
            let mut found = None;
            let mut l = d.l;
            while l > beg {
                l -= 1;
                if !is_dataline(&d.lines[l]) {
                    continue;
                }
                d.goto(l, 0);
                let f = d.get_field(Some(column), None);
                d.goto(l, 0);
                if n > 1 && nw(&f) {
                    n -= 1;
                } else if nw(&f) {
                    found = Some(trim(&f).to_owned());
                    break;
                }
            }
            match found {
                Some(f) => f,
                None => {
                    d.goto(l0, c0);
                    return Err("No non-empty field found".into());
                }
            }
        }
    };
    let mut next = None;
    if inc_on && n != 0 {
        let above = if numeric_inc || d.l == beg {
            None
        } else {
            d.forward_line(-1);
            let f = d.get_field(Some(column), None);
            nw(&f).then(|| trim(&f).to_owned())
        };
        next = Some(increment_field(&field, above.as_deref()));
    }
    d.goto(l0, c0);
    if initial.is_some() {
        let saved = d.st.may_need_update;
        d.st.may_need_update = false;
        next_row(d)?;
        d.st.may_need_update = saved;
        blank_field(d)?;
    }
    d.insert(next.as_deref().unwrap_or(&field));
    maybe_recalculate_line(d)?;
    d.align();
    Ok(())
}

// ---- rectangles ----

/// org-table-copy-region (and with `cut`, org-table-cut-region).
pub fn copy_region(d: &mut Doc, beg: (usize, usize), end: (usize, usize), cut: bool) -> R {
    let (a, b) = if beg <= end { (beg, end) } else { (end, beg) };
    d.goto(a.0, a.1);
    d.check_inside_data_field(false, false)?;
    let first = d.l;
    let c01 = d.current_column();
    d.goto(b.0, b.1);
    d.check_inside_data_field(false, true)?;
    let last = d.l;
    let c02 = d.current_column();
    let (cs, ce) = (c01.min(c02), c01.max(c02));
    let mut region = vec![];
    for l in first..=last {
        d.goto(l, 0);
        if d.at_hline() {
            continue;
        }
        let mut cols = vec![];
        for c in cs..=ce {
            cols.push(d.get_field(Some(c), cut.then_some("  ")));
        }
        region.push(cols);
    }
    if cut {
        d.align();
    }
    if d.interactive {
        d.message("Cells in the region copied, use M-x org-table-paste-rectangle to paste them in a table.");
    }
    d.st.put_clip(Some(region));
    Ok(())
}

/// org-table-paste-rectangle.
pub fn paste_rectangle(d: &mut Doc) -> R {
    let Some(clip) = d.st.clip().filter(|c| !c.is_empty()) else {
        return Err("First cut/copy a region to paste!".into());
    };
    d.check_inside_data_field(false, false)?;
    let column = d.current_column();
    let (l0, col0) = (d.l, d.current_column());
    let saved = d.st.no_realign;
    d.st.no_realign = true;
    let mut past_end = false;
    for row in clip {
        while !past_end && d.at_hline() {
            if d.l + 1 >= d.n() {
                past_end = true;
                d.eol();
            } else {
                d.goto(d.l + 1, 0);
            }
        }
        if past_end || (d.bolp() && !is_table_line(d.line())) {
            if !past_end {
                d.forward_line(-1);
            }
            d.eol();
            next_field(d)?;
            past_end = false;
        }
        let mut c = column;
        for field in &row {
            d.goto_column(c, false, true);
            d.get_field(None, Some(field));
            c += 1;
        }
        if d.l + 1 >= d.n() {
            past_end = true;
            d.eol();
        } else {
            d.goto(d.l + 1, 0);
        }
    }
    d.st.no_realign = saved;
    d.goto(l0, 0);
    d.goto_column(col0, false, false);
    d.align();
    Ok(())
}

// ---- create, convert, import ----

/// org-table-create: a `COLSxROWS` table at point.
pub fn create(d: &mut Doc, size: &str) -> R {
    static SEP: LazyLock<Regex> = LazyLock::new(|| re(r" *x *"));
    let split = split_string(size, &SEP);
    let rows: usize = split
        .get(1)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let columns: usize = split
        .first()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let indent = " ".repeat(d.current_col());
    let line = format!("{indent}|{}", "  |".repeat(columns));
    let (pos_l, pos_c) = (d.l, d.c);
    let start = if d.before().chars().all(|c| c == ' ' || c == '\t') {
        d.bol();
        pos_l
    } else {
        // newline: split the line at point.
        d.insert("\n");
        pos_l + 1
    };
    let mut new: Vec<String> = (0..rows).map(|_| line.clone()).collect();
    if rows > 1 {
        new.insert(1, format!("{indent}|-"));
    }
    let _ = pos_c;
    // Insert before the line at `start` (whose text stays after the table);
    // an empty last line is the end of the buffer.
    if start + 1 == d.n() && d.lines[start].is_empty() {
        d.lines.pop();
    }
    d.insert_lines(start, &new);
    d.goto(start, d.lines[start].find('|').map_or(0, |i| i + 1));
    if rows > 0 {
        d.align();
    }
    Ok(())
}

/// The separator argument of org-table-convert-region.
#[derive(Clone, Debug, PartialEq)]
pub enum Sep {
    Auto,
    BabelAuto,
    Comma,
    Tab,
    Spaces(i64),
    Regex(String),
}

/// org-table-convert-region on lines `beg..=end`.
pub fn convert_region(d: &mut Doc, beg: usize, end: usize, sep: Sep) -> R {
    let (beg, end) = (beg.min(end), beg.max(end));
    let max = opt_int("org-table-convert-region-max-lines", 999) as usize;
    if end - beg + 1 > max {
        return Err(format!(
            "Region is longer than `org-table-convert-region-max-lines' ({max}) lines; not converting"
        ));
    }
    let region: Vec<String> = d.lines[beg..=end].to_vec();
    let sep = match sep {
        Sep::Auto | Sep::BabelAuto => {
            if region.iter().all(|l| l.contains('\t')) {
                Sep::Tab
            } else if region.iter().all(|l| l.contains(',')) {
                Sep::Comma
            } else if sep == Sep::BabelAuto && region.len() == 1 {
                Sep::Regex(String::new())
            } else {
                Sep::Spaces(1)
            }
        }
        s => s,
    };
    let new: Vec<String> = if sep == Sep::Comma {
        region.iter().map(|l| csv_line(l)).collect()
    } else {
        let r = match &sep {
            Sep::Tab => re(r"^|\t"),
            Sep::Spaces(n) => {
                if *n < 1 {
                    return Err("Number of spaces in separator must be >= 1".into());
                }
                re(&format!(r"^ *| *\t *| {{{n},}}"))
            }
            Sep::Regex(s) if s.is_empty() => re(r"^ *"),
            Sep::Regex(s) => {
                let inner = super::emacs_regex_src(s);
                re(&format!(r"^ *|{inner}"))
            }
            _ => unreachable!(),
        };
        region
            .iter()
            .map(|l| r.replace_all(l, "| ").into_owned())
            .collect()
    };
    d.lines.splice(beg..=end, new);
    d.goto(beg, 0);
    d.align();
    Ok(())
}

/// One CSV line converted to a table row (the `(4)` separator branch).
fn csv_line(l: &str) -> String {
    let mut out = String::from("| ");
    let mut s = l;
    loop {
        let t = s.trim_start_matches([' ', '\t']);
        if t.is_empty() || t == "\r" {
            out.push_str(" |");
            break;
        }
        if let Some(q) = t.strip_prefix('"')
            && let Some(e) = q.find('"')
        {
            out.push_str(&q[..e].replace(['\n', '\r'], " "));
            s = &q[e + 1..];
            if s.starts_with('"') {
                out.push('"');
            }
            continue;
        }
        let n = s.find([',', '\r']).unwrap_or(s.len());
        if n > 0 {
            out.push_str(&s[..n]);
            s = &s[n..];
            continue;
        }
        match s.strip_prefix(',') {
            Some(r) => {
                out.push_str(" | ");
                s = r;
            }
            None => break,
        }
    }
    out
}

/// org-table-import: the file's lines converted at point.
pub fn import(d: &mut Doc, text: &str, sep: Sep) -> R {
    if !d.bolp() {
        d.insert("\n");
    }
    let lines: Vec<String> = text
        .strip_suffix('\n')
        .unwrap_or(text)
        .split('\n')
        .map(String::from)
        .collect();
    let n = lines.len();
    let at = d.l;
    if d.line().is_empty() {
        d.lines.splice(at..=at, lines);
    } else {
        d.insert_lines(at, &lines);
    }
    d.goto(at, 0);
    convert_region(d, at, at + n - 1, sep)
}

/// org-table-convert: Org table to table.el and back.
pub fn convert(d: &mut Doc) -> R {
    if d.at_table_el() {
        let (b, e) = d.table_el_bounds();
        let kept: Vec<String> = d.lines[b..e]
            .iter()
            .filter(|l| !l.trim_start().starts_with("+-"))
            .cloned()
            .collect();
        // table.el cells span lines; each line becomes a row.
        d.lines.splice(b..e, kept);
        d.goto(b, 0);
        return Ok(());
    }
    if !d.at_table() {
        return Ok(());
    }
    let (b, e) = (d.table_begin(), d.table_end());
    let rows: Vec<String> = d.lines[b..e]
        .iter()
        .filter(|l| !is_hline(l))
        .cloned()
        .collect();
    let rule = {
        let mut x = clean_line(&rows[0]);
        static BLANK: LazyLock<Regex> = LazyLock::new(|| re(r"\|( +)\|"));
        while let Some(m) = BLANK.captures(&x) {
            let r = m.get(0).unwrap().range();
            let n = m.get(1).unwrap().len();
            x.replace_range(r, &format!("+{}|", "-".repeat(n)));
        }
        let x = x.replacen('|', "+", 1);
        if let Some(stripped) = x.strip_suffix('|') {
            format!("{stripped}+")
        } else {
            x
        }
    };
    let mut new = vec![rule.clone()];
    for r in rows {
        new.push(r);
        new.push(rule.clone());
    }
    d.lines.splice(b..e, new);
    d.goto(b, 0);
    Ok(())
}

// ---- sorting, transposing, wrapping, summing ----

/// org-table-sort-lines with SORTING-TYPE `kind` (a n t f, capital
/// reverses). `key`/`cmp` are the ?f extractor and comparator.
pub fn sort_lines(
    d: &mut Doc,
    with_case: bool,
    kind: char,
    column: Option<usize>,
    key: Option<&dyn Fn(&str) -> SortKey>,
) -> R {
    if let Some(((l, c), _)) = d.region {
        d.goto(l, c);
    }
    {
        let (l0, c0) = (d.l, d.c);
        d.skip_back(|c| c == ' ' || c == '\t');
        if d.bolp()
            && let Some(i) = d.rest().find('|')
        {
            d.c += i + 1;
        }
        let r = d.check_inside_data_field(false, false);
        d.goto(l0, c0);
        r?;
    }
    let column = match d.current_column() {
        0 => column.unwrap_or(1),
        c => c,
    };
    let (start, end) = (d.table_begin(), d.table_end());
    let (lo, hi) = if let Some((a, b)) = d.region {
        (a.0.min(b.0), a.0.max(b.0) + 1)
    } else {
        let lo = (start..d.l)
            .rev()
            .find(|&l| is_hline(&d.lines[l]))
            .map_or(start, |l| l + 1);
        let hi = (d.l + 1..end)
            .find(|&l| is_hline(&d.lines[l]))
            .unwrap_or(end);
        // re-search-forward from point also matches the current line.
        let hi = if is_hline(&d.lines[d.l]) { d.l } else { hi };
        (lo, hi)
    };
    let coords = (d.l - lo, d.current_col());
    let shrunk: Vec<usize> = d.shrunk().into_iter().filter(|&c| c != column).collect();
    // Records: each data line with the non-data lines following it.
    let mut lead: Vec<String> = vec![];
    let mut recs: Vec<(SortKey, Vec<String>)> = vec![];
    for l in lo..hi {
        let line = d.lines[l].clone();
        if is_dataline(&line) {
            d.goto(l, 0);
            let f = d.get_field(Some(column), None);
            let f = trim(&f);
            let k = match kind.to_ascii_lowercase() {
                'n' => SortKey::Num(super::lisp::string_to_number(f).f()),
                'a' => SortKey::Str(if with_case {
                    f.to_owned()
                } else {
                    f.to_lowercase()
                }),
                't' => SortKey::Num(time_key(f)),
                'f' => match key {
                    Some(k) => k(f),
                    None => return Err("Missing key extractor to sort rows".into()),
                },
                _ => return Err(format!("Invalid sorting type `{kind}'")),
            };
            recs.push((k, vec![line]));
        } else if let Some(last) = recs.last_mut() {
            last.1.push(line);
        } else {
            lead.push(line);
        }
    }
    let rev = kind.is_ascii_uppercase();
    // sort-subr: stable; reverse means the records are reversed, sorted,
    // and reversed again (equal keys keep reverse order).
    if rev {
        recs.reverse();
    }
    recs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    if rev {
        recs.reverse();
    }
    let mut new = lead;
    for (_, ls) in recs {
        new.extend(ls);
    }
    d.lines.splice(lo..hi, new);
    let (b, e) = (start, end);
    d.st.set_shrunk(b, e, shrunk);
    d.goto(lo + coords.0, 0);
    d.move_to_column(coords.1);
    Ok(())
}

/// A sort key.
#[derive(Clone, Debug, PartialEq, PartialOrd)]
pub enum SortKey {
    Num(f64),
    Str(String),
}

lazy_re!(
    TS_BOTH,
    r"[\[<]([0-9]{4})-([0-9]{2})-([0-9]{2})(?: [^]>\s0-9+-]+)?(?: ([0-9]{1,2}):([0-9]{2}))?"
);
lazy_re!(HHMM, r"\b([0-9]+):([0-9]{2})\b");

/// The ?t key: a timestamp's time, else a duration in minutes.
fn time_key(f: &str) -> f64 {
    if let Some(m) = TS_BOTH.captures(f) {
        let days = super::days_from_civil(
            m[1].parse().unwrap_or(0),
            m[2].parse().unwrap_or(1),
            m[3].parse().unwrap_or(1),
        );
        let h: f64 = m.get(4).map_or(0.0, |x| x.as_str().parse().unwrap_or(0.0));
        let mi: f64 = m.get(5).map_or(0.0, |x| x.as_str().parse().unwrap_or(0.0));
        return days as f64 * 86400.0 + h * 3600.0 + mi * 60.0;
    }
    if let Some(m) = super::duration_minutes(f) {
        return m;
    }
    if let Some(m) = HHMM.captures(f) {
        return m[1].parse::<f64>().unwrap_or(0.0) * 60.0 + m[2].parse::<f64>().unwrap_or(0.0);
    }
    0.0
}

/// org-table-transpose-table-at-point.
pub fn transpose(d: &mut Doc) -> R {
    let table: Vec<Vec<String>> = d
        .to_lisp()
        .into_iter()
        .filter_map(|r| match r {
            Row::Cells(c) => Some(c),
            Row::Hline => None,
        })
        .collect();
    let dline_old = d.current_line_index();
    let col_old = d.current_column();
    let ncols = table.first().map_or(0, Vec::len);
    let contents: Vec<Vec<String>> = (0..ncols)
        .map(|j| {
            table
                .iter()
                .map(|row| row.get(j).cloned().unwrap_or_default())
                .collect()
        })
        .collect();
    let (b, e) = (d.table_begin(), d.table_end());
    let indent = d.lines[b][..d.lines[b].find('|').unwrap_or(0)].to_owned();
    let new: Vec<String> = contents
        .iter()
        .map(|x| format!("{indent}| {}  |", x.join(" | ")))
        .collect();
    d.lines.splice(b..e, new);
    d.goto(b, 0);
    d.goto_line(col_old);
    d.goto_column(dline_old, false, false);
    d.align();
    Ok(())
}

/// org-wrap: fill `s` into at most `nlines` lines of minimal width.
pub fn wrap_text(s: &str, nlines: usize) -> Vec<String> {
    let words: Vec<&str> = s.split_whitespace().collect();
    if words.is_empty() {
        return vec![];
    }
    let maxw = words.iter().map(|w| width(w)).max().unwrap_or(1);
    let fill = |w: usize| -> Vec<String> {
        let mut lines: Vec<String> = vec![];
        let mut cur = String::new();
        for word in &words {
            if !cur.is_empty() && width(&cur) + 1 + width(word) > w {
                lines.push(std::mem::take(&mut cur));
            }
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(word);
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
        lines
    };
    let total = width(s.trim());
    let mut w = maxw.max(total.div_ceil(nlines.max(1)));
    loop {
        let l = fill(w);
        if l.len() <= nlines.max(1) || w >= total {
            return l;
        }
        w += 1;
    }
}

/// org-table-wrap-region.
pub fn wrap_region(d: &mut Doc, arg: Option<i64>) -> R {
    d.check_inside_data_field(false, false)?;
    if let Some((a, b)) = d.region {
        let clip_save = d.st.take_clip();
        copy_region(d, a, b, true)?;
        let clip = d.st.take_clip().unwrap_or_default();
        if clip.first().is_some_and(|r| r.len() > 1) {
            d.st.put_clip(clip_save);
            return Err("Region must be limited to single column".into());
        }
        let n = clip.len() as i64;
        let nlines = match arg {
            None => n,
            Some(a) if a < 1 => n + a,
            Some(a) => a,
        };
        let text = clip
            .iter()
            .map(|r| trim(&r[0]).to_owned())
            .collect::<Vec<_>>()
            .join(" ");
        let wrapped: Vec<Vec<String>> = wrap_text(&text, nlines.max(1) as usize)
            .into_iter()
            .map(|l| vec![l])
            .collect();
        d.st.put_clip(Some(wrapped));
        let start = if a <= b { a } else { b };
        d.goto(start.0, start.1);
        paste_rectangle(d)?;
        d.st.put_clip(clip_save);
        d.align();
        return Ok(());
    }
    if !super::super::options::get("org-M-RET-may-split-line").is_some_and(|v| match v {
        toml::Value::Boolean(b) => b,
        toml::Value::Array(a) => a
            .iter()
            .any(|e| e.as_str() == Some("table") || e.as_str() == Some("default")),
        _ => true,
    }) && super::super::options::get("org-M-RET-may-split-line").is_some()
    {
        d.skip_fwd(|c| c != '|');
    }
    if arg.is_some() {
        let s = blank_field(d)?.unwrap_or_default();
        let col = d.current_column();
        d.forward_line(-1);
        while d.at_hline() && d.l > 0 {
            d.forward_line(-1);
        }
        d.goto_column(col, false, false);
        d.skip_fwd(|c| c != '|');
        d.skip_back(|c| c == ' ');
        d.insert(&format!(" {}", trim(&s)));
        d.align();
    } else if let Some(i) = d.rest().find('|').filter(|&i| i > 0) {
        let s = d.rest()[..i].to_owned();
        let c = d.c;
        d.replace_in_line(c..c + i + 1, " |");
        d.c = c;
        next_row(d)?;
        d.insert(&format!("{} ", trim(&s)));
        d.align();
    } else {
        next_row(d)?;
    }
    Ok(())
}

lazy_re!(HMS_SUM, r"^([0-9]+):([0-9]+)(:([0-9]+))?$");
lazy_re!(ZEROISH, r"^[-+ \t0.edED]+$");

/// org-table--number-for-summing: (value, is a time).
fn number_for_summing(s: &str) -> Option<(f64, bool)> {
    let s = s
        .trim_start_matches(' ')
        .trim_start_matches('|')
        .trim_start_matches(' ');
    let s = s
        .trim_end_matches(' ')
        .trim_end_matches('|')
        .trim_end_matches(' ');
    let n = super::lisp::string_to_number(s).f();
    if s.contains('0') && ZEROISH.is_match(s) {
        return Some((0.0, false));
    }
    if !s.is_empty() && s.chars().all(|c| c == ' ' || c == '\t') {
        return None;
    }
    if let Some(m) = HMS_SUM.captures(s) {
        let h: f64 = m[1].parse().unwrap_or(0.0);
        let mi: f64 = m[2].parse().unwrap_or(0.0);
        let se: f64 = m.get(4).map_or(0.0, |x| x.as_str().parse().unwrap_or(0.0));
        return Some((h + mi / 60.0 + se / 3600.0, true));
    }
    if n == 0.0 { None } else { Some((n, false)) }
}

/// org-table-sum: the sum as a string (also put on the kill ring).
pub fn sum(d: &mut Doc, nlast: Option<usize>) -> Result<String, String> {
    let (l0, c0) = (d.l, d.c);
    let (beg, end) = match d.region {
        Some(r) => r,
        None => {
            let col = d.current_column();
            let b = d.table_begin();
            let e = d.table_end();
            let Some(first) = (b..e).find(|&l| is_dataline(&d.lines[l])) else {
                return Err("No table data".into());
            };
            let last = (b..e).rev().find(|&l| is_dataline(&d.lines[l])).unwrap();
            d.goto(first, 0);
            d.goto_column(col, false, false);
            let a = (d.l, d.c);
            d.goto(last, 0);
            d.goto_column(col, false, false);
            (a, (d.l, d.c))
        }
    };
    let clip_save = d.st.take_clip();
    let interactive = d.interactive;
    d.interactive = false;
    let r = copy_region(d, beg, end, false);
    d.interactive = interactive;
    let items: Vec<String> =
        d.st.take_clip()
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .collect();
    d.st.put_clip(clip_save);
    d.goto(l0, c0);
    r?;
    let items: Vec<String> = match nlast {
        Some(n) if n < items.len() => items[items.len() - n..].to_vec(),
        _ => items,
    };
    let mut timecnt = 0;
    let numbers: Vec<f64> = items
        .iter()
        .filter_map(|s| number_for_summing(s))
        .map(|(v, t)| {
            if t {
                timecnt += 1;
            }
            v
        })
        .collect();
    let res: f64 = numbers.iter().sum();
    let sres = if timecnt == 0 {
        super::lisp::num_to_string(&super::lisp::Num::from_f64_sum(res, &items))
    } else {
        let diff = 3600.0 * res;
        let h = (diff / 3600.0).floor();
        let diff = diff.rem_euclid(3600.0);
        let m = (diff / 60.0).floor();
        let s = diff.rem_euclid(60.0);
        format!("{h:.0}:{m:02.0}:{s:02.0}")
    };
    d.kill = Some((sres.clone(), false));
    if d.interactive {
        d.message(format!(
            "Sum of {} items: {:<20}     (p will insert result into buffer)",
            numbers.len(),
            sres
        ));
    }
    Ok(sres)
}

/// org-table-justify-field-maybe.
pub fn justify_field_maybe(d: &mut Doc, new: Option<&str>) {
    let mut new = new.map(str::to_owned);
    if let Some(n) = &new
        && n.contains('\n')
    {
        d.message(format!("Removing newlines from formula result: {n:?}"));
        new = Some(n.trim_matches('\n').replace('\n', " "));
    }
    if new.is_none() && d.st.may_need_update {
        return;
    }
    if d.at_hline() {
        return;
    }
    let (b, e) = (d.table_begin(), d.table_end());
    if d.st.aligned != Some((b, e)) || !(b..e).contains(&d.l) {
        d.st.may_need_update = true;
        d.st.last_alignment.clear();
        d.st.last_widths.clear();
    }
    let Some(new) = new else { return };
    let col = d.current_column();
    if col == 0 {
        return;
    }
    let (l0, c0) = (d.l, d.c);
    d.skip_back(|c| c != '|');
    static CELL: LazyLock<Regex> = LazyLock::new(|| re(r"^ *(?:([^|]*?) *(\|)|([^|]+?) *($))"));
    let Some(m) = CELL.captures(d.rest()) else {
        d.st.may_need_update = true;
        d.goto(l0, c0);
        return;
    };
    let cell_range = m.get(0).unwrap().range();
    let cell = m.get(0).unwrap().as_str().to_owned();
    let field = m.get(1).map_or("", |x| x.as_str()).to_owned();
    let closed = m.get(2).is_some_and(|x| !x.is_empty());
    let align = d.st.last_alignment.get(col - 1).copied().unwrap_or('l');
    let w = d.st.last_widths.get(col - 1).copied();
    let new_cell = if d.st.may_need_update {
        format!(" {new} |")
    } else if !closed {
        d.st.may_need_update = true;
        format!(" {new} |")
    } else if let Some(w) = w.filter(|&w| width(&new) <= w) {
        format!("{}|", super::doc::align_field(&new, w, align))
    } else {
        d.st.may_need_update = true;
        format!(" {new} |")
    };
    let _ = field;
    if new_cell != cell {
        let base = d.c;
        d.replace_in_line(base + cell_range.start..base + cell_range.end, &new_cell);
    }
    d.goto(l0, c0);
}

/// org-table-rotate-recalc-marks.
pub fn rotate_recalc_marks(d: &mut Doc, newchar: Option<char>) -> R {
    if !d.at_table() {
        return Err("Not at a table".into());
    }
    const MARKS: &[(char, &str)] = &[
        (' ', "Unmarked: no special line, no automatic recalculation"),
        (
            '#',
            "Automatically recalculate this line upon TAB, RET, and C-c C-c in the line",
        ),
        (
            '*',
            "Recalculate only when entire table is recalculated with `C-u C-c *'",
        ),
        (
            '!',
            "Column name definition line.  Reference in formula as $name.",
        ),
        (
            '$',
            "Parameter definition line name=value.  Reference in formula as $name.",
        ),
        ('_', "Names for values in row below this one."),
        ('^', "Names for values in row above this one."),
    ];
    let region = d.region.map(|(a, b)| (a.0.min(b.0), a.0.max(b.0)));
    let l = d.l;
    let col = d.current_column();
    if let Some(c) = newchar
        && !MARKS.iter().any(|(m, _)| *m == c)
    {
        return Err(format!(
            "Invalid character `{c}' in `org-table-rotate-recalc-marks'"
        ));
    }
    let (b, e) = (d.table_begin(), d.table_end());
    static SPECIAL: LazyLock<Regex> =
        LazyLock::new(|| re(r"^[ \t]*\|[^-|][^|]*[^#!$*_^| \t][^|]*\|"));
    let no_special = (b..e).any(|i| SPECIAL.is_match(&d.lines[i]));
    if let Some((l1, _)) = region {
        d.goto(l1, 0);
    }
    if !is_dataline(d.line()) {
        return Err("Not at a table data line".into());
    }
    if no_special {
        d.goto_column(1, false, false);
        insert_column(d)?;
    }
    let prev_len = d.line().len();
    static MARK: LazyLock<Regex> = LazyLock::new(|| re(r"^[ \t]*\| *([#!$*^_ ]) *\|"));
    let ch = match MARK.captures(d.line()) {
        None => '#',
        Some(_) if newchar.is_some() => newchar.unwrap(),
        Some(m) => {
            let cur = m[1].chars().next().unwrap();
            let order: Vec<char> = MARKS.iter().map(|(c, _)| *c).chain([' ']).collect();
            let i = order.iter().position(|&c| c == cur).unwrap();
            order[i + 1]
        }
    };
    let line_now = d.l;
    d.get_field(Some(1), Some(&format!(" {ch} ")));
    if let Some((_, l2)) = region {
        for i in line_now + 1..=l2.min(d.n() - 1) {
            if is_dataline(&d.lines[i]) {
                d.goto(i, 0);
                d.get_field(Some(1), Some(&format!(" {ch} ")));
            }
        }
    }
    d.goto(line_now, 0);
    if d.line().len() != prev_len || region.is_some() {
        d.align();
    }
    d.goto(l, 0);
    d.goto_column(if no_special { col + 1 } else { col }, false, false);
    if d.interactive {
        let m = MARKS.iter().find(|(c, _)| *c == ch).map_or("", |(_, m)| *m);
        d.message(m);
    }
    Ok(())
}

/// The table at point as text lines, for exports.
pub fn table_lines(d: &Doc) -> Vec<String> {
    d.lines[d.table_begin()..d.table_end()].to_vec()
}

/// org-table-default-size.
pub fn default_size() -> String {
    opt_string("org-table-default-size", "5x2")
}

pub fn field_spec_re() -> &'static Regex {
    &FIELD_SPEC
}
