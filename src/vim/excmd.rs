//! Ex commands that need the whole editor (registers, options, Normal
//! mode, marks): the `EDITOR` entries of `ex::cmd::COMMANDS`, run here
//! once the ex interpreter has read their range.

use super::{marks, ops, register};
use crate::editor::{Editor, Mode, Register};
use crate::ex::addr::Range;

/// What the ex interpreter read for an `EDITOR` command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    pub name: &'static str,
    pub range: Option<Range>,
    /// The range, or the cursor's line.
    pub at: Range,
    pub force: bool,
    pub arg: String,
}

/// Commands that leave the text alone (and so run in generated buffers).
const READ_ONLY: &[&str] = &[
    "abbreviate",
    "iabbrev",
    "cabbrev",
    "noreabbrev",
    "inoreabbrev",
    "cnoreabbrev",
    "unabbreviate",
    "iunabbrev",
    "cunabbrev",
    "abclear",
    "iabclear",
    "cabclear",
    "map",
    "nmap",
    "vmap",
    "xmap",
    "smap",
    "omap",
    "imap",
    "cmap",
    "noremap",
    "nnoremap",
    "vnoremap",
    "xnoremap",
    "snoremap",
    "onoremap",
    "inoremap",
    "cnoremap",
    "unmap",
    "nunmap",
    "vunmap",
    "xunmap",
    "sunmap",
    "ounmap",
    "iunmap",
    "cunmap",
    "mapclear",
    "nmapclear",
    "vmapclear",
    "xmapclear",
    "smapclear",
    "omapclear",
    "imapclear",
    "cmapclear",
    "yank",
    "registers",
    "display",
    "marks",
    "jumps",
    "changes",
    "=",
    "print",
    "number",
    "#",
    "list",
    "k",
    "mark",
    "nohlsearch",
    "stopinsert",
    "delmarks",
    "clearjumps",
    "undolist",
    "wundo",
];

pub fn run(ed: &mut Editor, c: Call) {
    if ed.generated() && !READ_ONLY.contains(&c.name) {
        ed.set_err("generated Git buffer is read-only");
        return;
    }
    let r = match c.name {
        "normal" => {
            normal(ed, &c);
            Ok(())
        }
        "yank" | "delete" => yank_delete(ed, &c),
        "put" => put(ed, &c),
        "<" | ">" => shift(ed, &c),
        "k" | "mark" => mark(ed, &c),
        "undo" if c.force || !c.arg.is_empty() => undo_n(ed, &c),
        "earlier" | "later" => later(ed, &c),
        "undojoin" => ed.undo.join_next(),
        "undolist" => {
            undolist(ed);
            Ok(())
        }
        "wundo" => wundo(ed, &c),
        "rundo" => rundo(ed, &c),
        "undo" | "redo" => {
            let redo = c.name == "redo";
            let p = if redo {
                ed.undo.redo(&mut ed.buf)
            } else {
                ed.undo.undo(&mut ed.buf)
            };
            match p {
                Some(p) => ed.set_cursor(p.0, p.1),
                None if redo => ed.set_err("Already at newest change"),
                None => ed.set_err("Already at oldest change"),
            }
            Ok(())
        }
        "startinsert" | "startreplace" => {
            if c.force {
                ed.cur.byte = ed.buf.line_len(ed.cur.line);
            }
            ed.mode = Mode::Insert;
            if c.name == "startreplace" {
                ed.vim.replace = Some(vec![]);
            }
            Ok(())
        }
        "stopinsert" => {
            if ed.mode == Mode::Insert {
                super::insert::leave(ed);
            }
            Ok(())
        }
        "nohlsearch" => Ok(()),
        "retab" => retab(ed, &c),
        "left" | "center" | "right" => align(ed, &c),
        "registers" | "display" => {
            registers(ed, &c.arg);
            Ok(())
        }
        "marks" => {
            list_marks(ed, &c.arg);
            Ok(())
        }
        "delmarks" => delmarks(ed, &c),
        "jumps" | "changes" => {
            list_positions(ed, c.name == "jumps");
            Ok(())
        }
        "clearjumps" => {
            marks::clear_jumps(ed);
            Ok(())
        }
        "=" => {
            let n = match c.range {
                Some(r) => r.end + 1,
                None => ed.line_count(),
            };
            ed.set_msg(n.to_string());
            Ok(())
        }
        "print" | "number" | "#" | "list" => print(ed, &c),
        n if n.contains("map") => super::mappings::command(ed, n, c.force, &c.arg),
        n if n.contains("ab") => super::mappings::abbrev_command(ed, n, &c.arg),
        "source" => source(ed, &c.arg, c.force),
        "update" => {
            if ed.buf.modified {
                ed.pending_effect = Some(crate::ex::ExEffect::Write {
                    path: (!c.arg.is_empty()).then(|| c.arg.clone()),
                    force: c.force,
                    range: c.range,
                    then_quit: false,
                });
            }
            Ok(())
        }
        _ => Err(format!("E492: Not an editor command: {}", c.name)),
    };
    if let Err(e) = r {
        ed.set_err(e);
    }
}

/// `[x] [count]`: a register name and a count after a command; the count
/// makes the range start at its last line.
fn reg_count(c: &Call, takes_reg: bool) -> Result<(Option<char>, Range), String> {
    let mut arg = c.arg.trim();
    let mut reg = None;
    if takes_reg
        && let Some(ch) = arg.chars().next()
        && !ch.is_ascii_digit()
    {
        if !register::valid(ch) || matches!(ch, '.' | ':' | '%' | '/') {
            return Err(format!("E488: Trailing characters: {arg}"));
        }
        reg = Some(ch);
        arg = arg[ch.len_utf8()..].trim_start();
    }
    let mut r = c.at;
    if !arg.is_empty() {
        let n: usize = arg
            .parse()
            .map_err(|_| format!("E488: Trailing characters: {arg}"))?;
        if n == 0 {
            return Err("E939: Positive count required".into());
        }
        r = Range {
            start: r.end,
            end: r.end + n - 1,
        };
    }
    Ok((reg, r))
}

/// `:normal[!] {commands}`: the keys as typed, once per line of the range
/// (each from column 0) or once at the cursor. An unfinished command ends
/// as if Esc were typed.
fn normal(ed: &mut Editor, c: &Call) {
    let keys = register::text_to_keys(c.arg.trim_start());
    if keys.is_empty() {
        return;
    }
    let lines: Vec<Option<usize>> = match c.range {
        Some(r) => (r.start..=r.end).map(Some).collect(),
        None => vec![None],
    };
    for l in lines {
        if let Some(l) = l {
            let l = l.min(ed.line_count() - 1);
            ed.set_cursor(l, 0);
        }
        for &k in &keys {
            // Without `!` the keys are mapped.
            if c.force {
                ed.handle_key(k);
            } else {
                super::mappings::typed(ed, k);
            }
        }
        super::mappings::flush(ed);
        // End what the keys left unfinished (Visual mode stays).
        ed.vim.pending.clear();
        match ed.mode {
            Mode::Insert => super::insert::leave(ed),
            Mode::Command(_) => {
                ed.mode = Mode::Normal;
                ed.clamp_cursor();
            }
            _ => {}
        }
    }
}

/// `:[range]y[ank] [x] [count]` and `:[range]d[elete] [x] [count]`.
fn yank_delete(ed: &mut Editor, c: &Call) -> Result<(), String> {
    let (reg, r) = reg_count(c, true)?;
    let end = r.end.min(ed.line_count() - 1);
    let text = (r.start..=end)
        .map(|l| ed.buf.line(l))
        .collect::<Vec<_>>()
        .join("\n");
    ed.vim.register = reg;
    let delete = c.name == "delete";
    register::store(
        ed,
        Register {
            text,
            linewise: true,
            block: false,
        },
        !delete,
        true,
    );
    ed.vim.register = None;
    if delete {
        ops::splice_lines(ed, r.start, end - r.start + 1, &[]);
        let l = r.start.min(ed.line_count() - 1);
        ed.set_cursor(l, ed.first_nonblank(l));
    }
    Ok(())
}

/// `:[line]pu[t][!] [x]`: a register's text as lines below the line (above
/// with `!`; `:0put` above the first). The cursor goes to the last new
/// line's first non-blank.
fn put(ed: &mut Editor, c: &Call) -> Result<(), String> {
    let arg = c.arg.trim();
    let name = arg.chars().next();
    if arg.chars().count() > 1 {
        return Err(format!("E488: Trailing characters: {arg}"));
    }
    let reg = match name {
        Some(ch) => register::get(ed, ch),
        None => Some(ed.reg.clone()),
    };
    let Some(reg) = reg else {
        return Err(format!("E353: Nothing in register {}", name.unwrap_or('"')));
    };
    let lines: Vec<String> = reg.text.split('\n').map(String::from).collect();
    let at = if c.force { c.at.end } else { c.at.end + 1 };
    let n = lines.len();
    ops::splice_lines(ed, at, 0, &lines);
    let last = at + n - 1;
    ed.set_cursor(last, ed.first_nonblank(last));
    Ok(())
}

/// `:[range]> [count]` (`:>>` shifts twice).
fn shift(ed: &mut Editor, c: &Call) -> Result<(), String> {
    let ch = c.name.chars().next().unwrap_or('>');
    let more = c.arg.trim_start().chars().take_while(|&x| x == ch).count();
    let rest = c.arg.trim_start()[more..].to_string();
    let c2 = Call {
        arg: rest,
        ..c.clone()
    };
    let (_, r) = reg_count(&c2, false)?;
    let end = r.end.min(ed.line_count() - 1);
    ops::shift(ed, r.start, end, ch == '<', 1 + more);
    ed.set_cursor(end, ed.first_nonblank(end));
    Ok(())
}

/// `:[range]k{a-z}` and `:[range]ma[rk] {a-z}`: at the range's last line.
fn mark(ed: &mut Editor, c: &Call) -> Result<(), String> {
    let arg = c.arg.trim();
    let mut it = arg.chars();
    let (Some(ch), None) = (it.next(), it.next()) else {
        return Err(if arg.is_empty() {
            "E471: Argument required".into()
        } else {
            "E488: Trailing characters".into()
        });
    };
    if !(ch.is_ascii_alphabetic() || "'`[]<>".contains(ch)) {
        return Err("E191: Argument must be a letter or forward/backward quote".into());
    }
    marks::set(ed, ch, (c.at.end, 0));
    Ok(())
}

/// `:[range]ret[ab][!] [tabstop]`: white space with a tab in it (with `!`
/// any run of spaces too) redone for the new 'tabstop' and 'expandtab';
/// then 'tabstop' is the new one.
fn retab(ed: &mut Editor, c: &Call) -> Result<(), String> {
    use crate::options::{Level, Opt, Value};
    let old_ts = ed.tabstop();
    let new_ts = match c.arg.trim() {
        "" => old_ts,
        a => match a.parse::<usize>() {
            Ok(0) | Err(_) => return Err("E475: Invalid argument".into()),
            Ok(n) => n,
        },
    };
    let et = ed.opts.bool(Opt::Expandtab);
    // Without a range: the whole buffer.
    let r = c.range.unwrap_or(Range {
        start: 0,
        end: ed.line_count() - 1,
    });
    let mut changed = false;
    let mut out = vec![];
    for l in r.start..=r.end.min(ed.line_count() - 1) {
        let line = ed.buf.line(l);
        let new = retab_line(&line, old_ts, new_ts, et, c.force);
        changed |= new != line;
        out.push(new);
    }
    if changed {
        ops::splice_lines(ed, r.start, out.len(), &out);
    }
    if new_ts != old_ts {
        let _ = ed
            .opts
            .put(Opt::Tabstop, Value::Num(new_ts as i64), Level::Local);
    }
    ed.set_line_keep_col(ed.cur.line);
    ed.clamp_cursor();
    Ok(())
}

fn retab_line(line: &str, old_ts: usize, new_ts: usize, et: bool, force: bool) -> String {
    let mut out = String::new();
    let (mut vcol, mut start_vcol) = (0, 0);
    let (mut spaces, mut got_tab) = (0, false);
    let mut run = String::new();
    let flush = |out: &mut String,
                 run: &mut String,
                 start: usize,
                 end: usize,
                 got_tab: bool,
                 spaces: usize| {
        if got_tab || (force && spaces > 1) {
            let mut n = end - start;
            let mut tabs = 0;
            if !et {
                let first = new_ts - start % new_ts;
                if n >= first {
                    n -= first;
                    tabs += 1;
                }
                tabs += n / new_ts;
                n %= new_ts;
            }
            out.push_str(&"\t".repeat(tabs));
            out.push_str(&" ".repeat(n));
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for ch in line.chars() {
        if ch == ' ' || ch == '\t' {
            if run.is_empty() {
                start_vcol = vcol;
            }
            run.push(ch);
            if ch == ' ' {
                spaces += 1;
                vcol += 1;
            } else {
                got_tab = true;
                vcol += old_ts - vcol % old_ts;
            }
        } else {
            if !run.is_empty() {
                flush(&mut out, &mut run, start_vcol, vcol, got_tab, spaces);
            }
            got_tab = false;
            spaces = 0;
            out.push(ch);
            vcol += crate::text::grapheme_width(ch.encode_utf8(&mut [0; 4]), vcol, old_ts).max(1);
        }
    }
    if !run.is_empty() {
        flush(&mut out, &mut run, start_vcol, vcol, got_tab, spaces);
    }
    out
}

/// `:[range]le[ft] [indent]`, `:ce[nter] [width]`, `:ri[ght] [width]`.
fn align(ed: &mut Editor, c: &Call) -> Result<(), String> {
    let n: Option<usize> = match c.arg.trim() {
        "" => None,
        a => Some(a.parse().map_err(|_| "E474: Invalid argument")?),
    };
    let width = n.unwrap_or(80);
    let (ts, et) = (ed.tabstop(), ed.opts.bool(crate::options::Opt::Expandtab));
    let indent = |cols: usize| {
        if et {
            " ".repeat(cols)
        } else {
            "\t".repeat(cols / ts) + &" ".repeat(cols % ts)
        }
    };
    let r = c.at;
    let end = r.end.min(ed.line_count() - 1);
    let mut out = vec![];
    for l in r.start..=end {
        let line = ed.buf.line(l);
        let text = line.trim_matches([' ', '\t']);
        let w = crate::text::display_width(text, ts, 0);
        let new = match c.name {
            "left" => indent(n.unwrap_or(0)) + line.trim_start_matches([' ', '\t']),
            _ if text.is_empty() => String::new(),
            "center" => indent(width.saturating_sub(w) / 2) + text,
            _ => indent(width.saturating_sub(w)) + text,
        };
        out.push(new);
    }
    let line = ed.cur.line;
    ops::splice_lines(ed, r.start, out.len(), &out);
    ed.set_cursor(line, ed.first_nonblank(line));
    Ok(())
}

/// A register's text on one line, as `:registers` shows it.
fn shown(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' => "^J".to_string(),
            '\t' => "^I".to_string(),
            '\u{80}' => String::new(),
            c if (c as u32) < 0x20 => format!("^{}", char::from(c as u8 + b'@')),
            c => c.to_string(),
        })
        .collect()
}

/// `:reg[isters] [names]`: what the registers hold, in a list.
fn registers(ed: &mut Editor, which: &str) {
    let names = "\"0123456789abcdefghijklmnopqrstuvwxyz-.:%/+*";
    let rows: Vec<String> = names
        .chars()
        .filter(|c| which.is_empty() || which.contains(*c))
        .filter_map(|c| {
            let r = register::get(ed, c)?;
            if r.text.is_empty() && !r.linewise {
                return None;
            }
            let kind = if r.linewise { 'l' } else { 'c' };
            Some(format!("  {kind}  \"{c}   {}", shown(&r.text)))
        })
        .collect();
    crate::pick::choose(ed, "registers", rows, "");
}

/// `:marks [names]`: the marks, in a list; picking one goes there.
fn list_marks(ed: &mut Editor, which: &str) {
    let names = "'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ\"[]^.<>";
    let rows: Vec<String> = names
        .chars()
        .filter(|c| which.is_empty() || which.contains(*c))
        .filter_map(|c| {
            let (l, b) = marks::get(ed, c)?;
            Some(format!(
                " {c} {:>6} {:>4} {}",
                l + 1,
                b,
                ed.buf.line(l).trim()
            ))
        })
        .collect();
    go_from_list(ed, "marks", rows);
}

/// `:jumps` and `:changes`.
fn list_positions(ed: &mut Editor, jumps: bool) {
    let (list, idx) = marks::list(ed, jumps);
    let rows: Vec<String> = list
        .iter()
        .enumerate()
        .map(|(i, &(l, b))| {
            let far = i.abs_diff(idx);
            let line = ed.buf.line(l.min(ed.line_count() - 1));
            format!("{far:>4} {:>6} {:>4} {}", l + 1, b, line.trim())
        })
        .collect();
    go_from_list(ed, if jumps { "jumps" } else { "changes" }, rows);
}

/// A list of positions (line and column in its second and third words):
/// picking one goes there.
fn go_from_list(ed: &mut Editor, title: &str, rows: Vec<String>) {
    crate::pick::choose(ed, title, rows, "");
    ed.org_then = Some(Box::new(|ed, picked| {
        let mut w = picked.split_whitespace().skip(1);
        if let (Some(Ok(l)), Some(Ok(b))) = (
            w.next().map(str::parse::<usize>),
            w.next().map(str::parse::<usize>),
        ) {
            let l = l.saturating_sub(1).min(ed.line_count() - 1);
            ed.set_cursor(l, b);
        }
    }));
}

/// `:delm[arks] {marks}` (`a-d` ranges) and `:delm!` (all a-z).
fn delmarks(ed: &mut Editor, c: &Call) -> Result<(), String> {
    let arg = c.arg.trim();
    if c.force {
        if !arg.is_empty() {
            return Err("E474: Invalid argument".into());
        }
        ed.marks.retain(|m, _| !m.is_ascii_lowercase());
        return Ok(());
    }
    if arg.is_empty() {
        return Err("E471: Argument required".into());
    }
    let chars: Vec<char> = arg.chars().filter(|c| !c.is_whitespace()).collect();
    let mut i = 0;
    while i < chars.len() {
        let from = chars[i];
        let (lo, hi) = if chars.get(i + 1) == Some(&'-') && i + 2 < chars.len() {
            i += 3;
            (from, chars[i - 1])
        } else {
            i += 1;
            (from, from)
        };
        if lo > hi {
            return Err(format!("E475: Invalid argument: {lo}-{hi}"));
        }
        for m in lo..=hi {
            ed.marks.remove(&m);
            ed.mark_cols.remove(&m);
        }
    }
    Ok(())
}

/// `:p`, `:nu`, `:#`, `:l`: the range's last line in the message (the
/// cursor goes there).
fn print(ed: &mut Editor, c: &Call) -> Result<(), String> {
    let (_, r) = reg_count(c, false)?;
    let l = r.end.min(ed.line_count() - 1);
    let line = ed.buf.line(l);
    let text = match c.name {
        "list" => format!("{}$", shown(&line)),
        "number" | "#" => format!("{:>3} {line}", l + 1),
        _ => line,
    };
    ed.set_cursor(l, ed.first_nonblank(l));
    ed.set_msg(text);
    Ok(())
}

/// `:so[urce][!] {file}`: each line an ex command (`"` comments, `\`
/// continuing the line before), or with `!` Normal-mode keys.
fn source(ed: &mut Editor, file: &str, keys: bool) -> Result<(), String> {
    let path = match file.strip_prefix("~/") {
        Some(rest) => {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest)
        }
        None => std::path::PathBuf::from(file),
    };
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("E484: Can't open file {file}: {e}"))?;
    if keys {
        for k in register::text_to_keys(&text) {
            ed.handle_key(k);
        }
        return Ok(());
    }
    let mut lines: Vec<(usize, String)> = vec![];
    for (i, raw) in text.lines().enumerate() {
        let t = raw.trim_start();
        if let Some(cont) = t.strip_prefix('\\')
            && let Some(last) = lines.last_mut()
        {
            last.1.push_str(cont);
            continue;
        }
        lines.push((i + 1, t.to_string()));
    }
    let mut first_err = None;
    for (n, line) in lines {
        let cmd = line.trim_start_matches([':', ' ', '\t']);
        if cmd.is_empty() || cmd.starts_with('"') {
            continue;
        }
        ed.msg = None;
        ed.run_ex(cmd);
        if let Some((m, true)) = &ed.msg
            && first_err.is_none()
        {
            first_err = Some(format!("{file} line {n}: {}", m.trim_start_matches("? ")));
        }
    }
    match first_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Put the cursor where an undo-tree move left it, or say why it didn't
/// move ("Already at oldest change" is a message, E830 an error).
fn moved(ed: &mut Editor, r: Result<Option<(usize, usize)>, String>) -> Result<(), String> {
    match r {
        Ok(Some(p)) => ed.set_cursor(p.0, p.1),
        Ok(None) => {}
        Err(e) if e.starts_with('E') => return Err(e),
        Err(e) => ed.set_msg(e),
    }
    Ok(())
}

/// `g-`, `g+`, `:earlier`, `:later`.
pub fn time_travel(ed: &mut Editor, step: i64, unit: crate::undo::Unit) {
    let r = ed.undo.time_travel(&mut ed.buf, step, unit);
    if let Err(e) = moved(ed, r) {
        ed.set_err(e);
    }
}

/// `:undo N`, `:undo!`, `:undo! N`.
fn undo_n(ed: &mut Editor, c: &Call) -> Result<(), String> {
    let n = match c.arg.as_str() {
        "" => None,
        a => Some(
            a.parse::<u64>()
                .map_err(|_| format!("E488: Trailing characters: {a}"))?,
        ),
    };
    let r = match n {
        None => Ok(ed.undo.undo_forget(&mut ed.buf, 1)),
        Some(n) if c.force => ed.undo.undo_forget_to(&mut ed.buf, n),
        Some(n) => ed.undo.undo_to(&mut ed.buf, n),
    };
    moved(ed, r)
}

/// `:earlier {N}` / `:later {N}`, `{N}s m h d` by time, `{N}f` by writes.
fn later(ed: &mut Editor, c: &Call) -> Result<(), String> {
    use crate::undo::Unit;
    let a = c.arg.as_str();
    let digits = a.find(|ch: char| !ch.is_ascii_digit()).unwrap_or(a.len());
    let n: i64 = if digits == 0 {
        1
    } else {
        // As getdigits_int(): a count past an int is 0 (no move).
        a[..digits].parse::<i32>().map_or(0, i64::from)
    };
    let (unit, mul) = match &a[digits..] {
        "" => (Unit::Changes, 1),
        "s" if digits > 0 => (Unit::Seconds, 1),
        "m" if digits > 0 => (Unit::Seconds, 60),
        "h" if digits > 0 => (Unit::Seconds, 60 * 60),
        "d" if digits > 0 => (Unit::Seconds, 24 * 60 * 60),
        "f" if digits > 0 => (Unit::Writes, 1),
        _ => return Err(format!("E475: Invalid argument: {a}")),
    };
    let step = n.saturating_mul(mul);
    let step = if c.name == "earlier" { -step } else { step };
    let r = ed.undo.time_travel(&mut ed.buf, step, unit);
    moved(ed, r)
}

/// `:undolist`: the leaves of the undo tree; picking one goes there.
fn undolist(ed: &mut Editor) {
    let rows = ed.undo.list();
    if rows.is_empty() {
        ed.set_msg("Nothing to undo");
        return;
    }
    crate::pick::choose(ed, "number changes  when               saved", rows, "");
    ed.org_then = Some(Box::new(|ed, picked| {
        if let Some(n) = picked.split_whitespace().next() {
            ed.run_ex(&format!("undo {n}"));
        }
    }));
}

/// `:wundo[!] {file}`: write the undo tree. Without `!` a file that isn't
/// an undo file is left alone.
fn wundo(ed: &mut Editor, c: &Call) -> Result<(), String> {
    let path = crate::session::expand_tilde(&c.arg);
    crate::undo::write_file(&ed.undo, &path, &ed.buf.text(), c.force)
}

/// `:rundo {file}`: read an undo tree written for this text.
fn rundo(ed: &mut Editor, c: &Call) -> Result<(), String> {
    if ed.undo.open_len() > 0 {
        return Ok(());
    }
    let path = crate::session::expand_tilde(&c.arg);
    let data = std::fs::read_to_string(&path)
        .map_err(|_| format!("E822: Cannot open undo file for reading: {}", c.arg))?;
    match crate::undo::Undo::from_file(&data, &ed.buf.text()) {
        Ok(mut u) => {
            u.levels = ed.undo.levels;
            ed.undo = u;
            // An unmodified buffer is saved in the state read back.
            ed.saved_state = if ed.buf.modified {
                u64::MAX
            } else {
                ed.undo.state_id()
            };
        }
        Err(e) => ed.set_msg(e),
    }
    Ok(())
}
