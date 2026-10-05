//! Babel values as Emacs Lisp data (`org::sexp::Sexp`): printing like
//! `prin1`/`princ`, `org-babel-read`, `org-babel-script-escape`, the
//! number reader, a small evaluator for header-argument Lisp (with
//! `emacs --batch` for anything else), and Org table text conversion
//! (`org-table-to-lisp`, `org-table-align`, `org-table-convert-region`,
//! `orgtbl-to-generic`) used for results and variables.

use crate::org::sexp::{self, Sexp};
use std::cell::RefCell;

thread_local! {
    /// `*this*`: the value of the block a `:post` call processes.
    static THIS: RefCell<Sexp> = const { RefCell::new(Sexp::Nil) };
}

pub fn set_this(v: Sexp) -> Sexp {
    THIS.with(|t| std::mem::replace(&mut *t.borrow_mut(), v))
}

pub fn this() -> Sexp {
    THIS.with(|t| t.borrow().clone())
}

pub fn hline() -> Sexp {
    Sexp::Sym("hline".into())
}

pub fn is_hline(v: &Sexp) -> bool {
    matches!(v, Sexp::Sym(s) if s == "hline")
}

pub fn str(s: impl Into<String>) -> Sexp {
    Sexp::Str(s.into())
}

/// `(listp v)`: a proper list or nil.
pub fn listp(v: &Sexp) -> bool {
    matches!(v, Sexp::List(_) | Sexp::Nil | Sexp::Dotted(..))
}

/// The elements of a proper list (nil is empty).
pub fn items(v: &Sexp) -> Option<&[Sexp]> {
    match v {
        Sexp::List(x) => Some(x),
        Sexp::Nil => Some(&[]),
        _ => None,
    }
}

/// A list value from items (nil when empty).
pub fn list(v: Vec<Sexp>) -> Sexp {
    if v.is_empty() {
        Sexp::Nil
    } else {
        Sexp::List(v)
    }
}

/// `(cons a b)`.
pub fn cons(a: Sexp, b: Sexp) -> Sexp {
    match b {
        Sexp::Nil => Sexp::List(vec![a]),
        Sexp::List(mut v) => {
            v.insert(0, a);
            Sexp::List(v)
        }
        Sexp::Dotted(mut v, t) => {
            v.insert(0, a);
            Sexp::Dotted(v, t)
        }
        b => Sexp::Dotted(vec![a], Box::new(b)),
    }
}

/// `(car x)` and `(cdr x)` of a cons.
pub fn car_cdr(v: &Sexp) -> Option<(Sexp, Sexp)> {
    match v {
        Sexp::List(x) if !x.is_empty() => Some((x[0].clone(), list(x[1..].to_vec()))),
        Sexp::Dotted(x, t) if x.len() == 1 => Some((x[0].clone(), (**t).clone())),
        Sexp::Dotted(x, t) => Some((x[0].clone(), Sexp::Dotted(x[1..].to_vec(), t.clone()))),
        _ => None,
    }
}

/// Emacs's float printing: the shortest `%.Ng` (N from 15) that reads
/// back, with `.0` added to integral values.
pub fn float_str(f: f64) -> String {
    if f.is_nan() {
        return if f.is_sign_negative() {
            "-0.0e+NaN"
        } else {
            "0.0e+NaN"
        }
        .into();
    }
    if f.is_infinite() {
        return if f < 0.0 { "-1.0e+INF" } else { "1.0e+INF" }.into();
    }
    let mut s = String::new();
    for p in 15..=17 {
        s = fmt_g(f, p);
        if s.parse::<f64>().ok() == Some(f) {
            break;
        }
    }
    if !s.contains(['.', 'e']) {
        s.push_str(".0");
    }
    s
}

/// C's `%.{p}g`.
fn fmt_g(f: f64, p: usize) -> String {
    if f == 0.0 {
        return if f.is_sign_negative() {
            "-0".into()
        } else {
            "0".into()
        };
    }
    let e = format!("{:.*e}", p - 1, f);
    let (mant, exp) = e.split_once('e').unwrap_or((&e, "0"));
    let x: i32 = exp.parse().unwrap_or(0);
    if x < -4 || x >= p as i32 {
        let m = strip_zeros(mant);
        format!("{m}e{}{:02}", if x < 0 { '-' } else { '+' }, x.abs())
    } else {
        let digits = (p as i32 - 1 - x).max(0) as usize;
        strip_zeros(&format!("{f:.digits$}")).to_owned()
    }
}

fn strip_zeros(s: &str) -> &str {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s
    }
}

/// `prin1-to-string`.
pub fn prin1(v: &Sexp) -> String {
    let mut out = String::new();
    print(v, true, &mut out);
    out
}

/// `(format "%s" v)`.
pub fn princ(v: &Sexp) -> String {
    let mut out = String::new();
    print(v, false, &mut out);
    out
}

fn print(v: &Sexp, escape: bool, out: &mut String) {
    match v {
        Sexp::Nil => out.push_str("nil"),
        Sexp::T => out.push('t'),
        Sexp::Int(i) => out.push_str(&i.to_string()),
        Sexp::Float(f) => out.push_str(&float_str(*f)),
        Sexp::Str(s) if escape => {
            out.push('"');
            for c in s.chars() {
                if c == '"' || c == '\\' {
                    out.push('\\');
                }
                out.push(c);
            }
            out.push('"');
        }
        Sexp::Str(s) | Sexp::Sym(s) => out.push_str(s),
        Sexp::List(x) => {
            if escape && x.len() == 2 && x[0] == Sexp::Sym("quote".into()) {
                out.push('\'');
                print(&x[1], escape, out);
                return;
            }
            out.push('(');
            for (i, e) in x.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                print(e, escape, out);
            }
            out.push(')');
        }
        Sexp::Dotted(x, t) => {
            out.push('(');
            for e in x {
                print(e, escape, out);
                out.push(' ');
            }
            out.push_str(". ");
            print(t, escape, out);
            out.push(')');
        }
        Sexp::Vector(x) => {
            out.push('[');
            for (i, e) in x.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                print(e, escape, out);
            }
            out.push(']');
        }
    }
}

/// A string value as text (`(if (stringp v) v (format "%S" v))`).
pub fn echo(v: &Sexp) -> String {
    match v {
        Sexp::Str(s) => s.clone(),
        v => prin1(v),
    }
}

/// A header value as a string: strings and symbols as is, numbers printed.
pub fn as_string(v: &Sexp) -> Option<String> {
    match v {
        Sexp::Nil => None,
        Sexp::Str(s) | Sexp::Sym(s) => Some(s.clone()),
        v => Some(princ(v)),
    }
}

/// The Emacs reader on a number token, else None.
pub fn read_number(tok: &str) -> Option<Sexp> {
    let t = tok.trim();
    if t.is_empty() {
        return None;
    }
    let b = t.as_bytes();
    let (sign, rest) = match b[0] {
        b'+' | b'-' => (b[0] == b'-', &t[1..]),
        _ => (false, t),
    };
    if rest.is_empty() {
        return None;
    }
    let int_part = rest.strip_suffix('.').unwrap_or(rest);
    if !int_part.is_empty() && int_part.bytes().all(|c| c.is_ascii_digit()) {
        return match int_part.parse::<i64>() {
            Ok(i) => Some(Sexp::Int(if sign { -i } else { i })),
            Err(_) => t.parse::<f64>().ok().map(Sexp::Float),
        };
    }
    // Floats: digits . digits, or a mantissa with an exponent.
    let (mant, exp) = match rest.find(['e', 'E']) {
        Some(i) => (&rest[..i], Some(&rest[i + 1..])),
        None => (rest, None),
    };
    let mant_ok = match mant.split_once('.') {
        Some((a, b)) => {
            a.bytes().all(|c| c.is_ascii_digit())
                && b.bytes().all(|c| c.is_ascii_digit())
                && (!b.is_empty() || (exp.is_some() && !a.is_empty()))
        }
        None => !mant.is_empty() && mant.bytes().all(|c| c.is_ascii_digit()) && exp.is_some(),
    };
    let exp_ok = exp.is_none_or(|e| {
        let e = e.strip_prefix(['+', '-']).unwrap_or(e);
        !e.is_empty() && e.bytes().all(|c| c.is_ascii_digit())
    });
    if mant_ok && exp_ok && !mant.is_empty() && mant != "." {
        return t.parse::<f64>().ok().map(Sexp::Float);
    }
    None
}

/// org-babel--string-to-number.
pub fn string_to_number(s: &str) -> Option<Sexp> {
    let trimmed = s.trim();
    if trimmed.contains(char::is_whitespace) {
        return None;
    }
    // ^[0-9e.+ -]+$ on some line of S.
    let ok = s
        .split('\n')
        .any(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_digit() || "e.+ -".contains(c)));
    if !ok {
        return None;
    }
    match sexp::read_prefix(s) {
        Ok((v @ (Sexp::Int(_) | Sexp::Float(_)), _)) => Some(v),
        _ => read_number(s),
    }
}

/// org-babel-read: numbers, Lisp (unless inhibited), quoted strings, or
/// the string itself.
pub fn read(cell: &str, inhibit_lisp: bool) -> Result<Sexp, String> {
    if cell.trim().is_empty() {
        return Ok(str(cell));
    }
    if let Some(n) = string_to_number(cell) {
        return Ok(n);
    }
    if !inhibit_lisp && (cell.starts_with(['(', '\'', '`', '[']) || cell == "*this*") {
        return eval_lisp_text(cell);
    }
    let t = cell.trim();
    if t.len() >= 2
        && t.starts_with('"')
        && t.ends_with('"')
        && let Ok((Sexp::Str(s), rest)) = sexp::read_prefix(t)
        && rest.trim().is_empty()
    {
        return Ok(Sexp::Str(s));
    }
    Ok(str(cell))
}

/// org-babel-string-read: strip one level of `"` then read without Lisp.
pub fn string_read(cell: &str) -> Sexp {
    let t = cell.trim();
    let inner = if t.len() >= 3 && t.starts_with('"') && t.ends_with('"') && !cell.contains('\n') {
        &t[1..t.len() - 1]
    } else {
        cell
    };
    read(inner, true).unwrap_or_else(|_| str(inner))
}

/// Evaluate a Lisp form written as text: data and simple forms here,
/// anything else with `emacs --batch`.
pub fn eval_lisp_text(text: &str) -> Result<Sexp, String> {
    let form = sexp::read_prefix(text)
        .map(|(f, _)| f)
        .map_err(|e| format!("Invalid read syntax in {text:?}: {e}"))?;
    match eval_simple(&form) {
        Some(v) => Ok(v),
        None => emacs_eval(text.trim()),
    }
}

fn eval_simple(f: &Sexp) -> Option<Sexp> {
    match f {
        Sexp::Nil | Sexp::T | Sexp::Int(_) | Sexp::Float(_) | Sexp::Str(_) | Sexp::Vector(_) => {
            Some(f.clone())
        }
        Sexp::Sym(s) if s == "*this*" => Some(this()),
        Sexp::Sym(s) if s.starts_with(':') => Some(f.clone()),
        Sexp::Sym(s) => radix(s),
        Sexp::List(x) => {
            let head = x[0].sym()?;
            let args = &x[1..];
            match head {
                "quote" | "function" if args.len() == 1 && !has_comma(&args[0]) => {
                    Some(args[0].clone())
                }
                "identity" if args.len() == 1 => eval_simple(&args[0]),
                "list" => Some(list(
                    args.iter().map(eval_simple).collect::<Option<Vec<_>>>()?,
                )),
                "concat" => {
                    let mut s = String::new();
                    for a in args {
                        match eval_simple(a)? {
                            Sexp::Str(x) => s.push_str(&x),
                            Sexp::Nil => {}
                            _ => return None,
                        }
                    }
                    Some(Sexp::Str(s))
                }
                "+" | "*" | "-" => {
                    let v: Vec<i64> = args
                        .iter()
                        .map(|a| {
                            eval_simple(a)
                                .and_then(|v| v.int().filter(|_| matches!(v, Sexp::Int(_))))
                        })
                        .collect::<Option<_>>()?;
                    Some(Sexp::Int(match head {
                        "+" => v.iter().sum(),
                        "*" => v.iter().product(),
                        _ if v.len() == 1 => -v[0],
                        _ => v[0] - v[1..].iter().sum::<i64>(),
                    }))
                }
                _ => None,
            }
        }
        Sexp::Dotted(..) => None,
    }
}

fn has_comma(f: &Sexp) -> bool {
    match f {
        Sexp::Sym(s) => s == ",",
        Sexp::List(x) | Sexp::Vector(x) => x.iter().any(has_comma),
        Sexp::Dotted(x, t) => x.iter().any(has_comma) || has_comma(t),
        _ => false,
    }
}

/// `#o755`, `#x1F`, `#b101`.
fn radix(s: &str) -> Option<Sexp> {
    let (r, d) = match s.get(..2)? {
        "#o" => (8, &s[2..]),
        "#x" => (16, &s[2..]),
        "#b" => (2, &s[2..]),
        _ => return None,
    };
    i64::from_str_radix(d, r).ok().map(Sexp::Int)
}

/// The `emacs` program for Lisp evaluation.
pub fn emacs_program() -> String {
    crate::org::options::string("org-babel-emacs-program", "emacs")
}

/// Evaluate FORM in `emacs --batch -Q` and read its printed value.
pub fn emacs_eval(form: &str) -> Result<Sexp, String> {
    let expr = format!(
        "(let ((print-length nil) (print-level nil)) (princ (prin1-to-string (eval (car (read-from-string {})) t))))",
        prin1(&Sexp::Str(form.into()))
    );
    let out = std::process::Command::new(emacs_program())
        .args(["--batch", "-Q", "--eval", &expr])
        .output()
        .map_err(|e| format!("Cannot evaluate Lisp {form}: {} ({e})", emacs_program()))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("Lisp error in {form}: {}", err.trim()));
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    sexp::read(&text).map_err(|e| format!("Cannot read Lisp value {text:?}: {e}"))
}

fn script_escape_inner(s: &str) -> Result<String, String> {
    let (mut in_single, mut in_double, mut backslash) = (false, false, false);
    let mut out = String::new();
    for ch in s.chars() {
        if backslash {
            backslash = false;
            if in_single && ch == '\'' {
                out.push(ch);
            } else if ch == '"' {
                out.push_str(if in_single { "\\\\\\\"" } else { "\\\"" });
            } else if ch == '\\' {
                out.push_str("\\\\");
            } else {
                out.push_str("\\\\");
                out.push(ch);
            }
            continue;
        }
        let quoted = in_double || in_single;
        match ch {
            '[' | '{' => out.push(if quoted { ch } else { '(' }),
            ']' | '}' => out.push(if quoted { ch } else { ')' }),
            ',' => out.push(if quoted { ',' } else { ' ' }),
            '\'' => {
                if in_double {
                    out.push('\'');
                } else {
                    in_single = !in_single;
                    out.push('"');
                }
            }
            '"' => {
                if in_single {
                    out.push_str("\\\"");
                } else {
                    in_double = !in_double;
                    out.push('"');
                }
            }
            '\\' => {
                if !quoted {
                    return Err(
                        "Can't handle backslash outside string in `org-babel-script-escape'".into(),
                    );
                }
                backslash = true;
            }
            c => out.push(c),
        }
    }
    if in_single || in_double {
        return Err("Unterminated string in `org-babel-script-escape'".into());
    }
    Ok(out)
}

/// org-babel-script-escape: a script's printed list, tuple or string as
/// Lisp data.
pub fn script_escape(s: &str, force: bool) -> Sexp {
    let n = s.chars().count();
    let first = s.chars().next();
    let last = s.chars().last();
    let bracketed = n >= 2
        && matches!(
            (first, last),
            (Some('['), Some(']')) | (Some('{'), Some('}')) | (Some('('), Some(')'))
        );
    let escaped = if bracketed {
        script_escape_inner(s).map(|e| format!("'{e}"))
    } else if force
        || (n > 2
            && matches!(
                (first, last),
                (Some('\''), Some('\'')) | (Some('"'), Some('"'))
            ))
    {
        script_escape_inner(s)
    } else {
        Ok(s.to_owned())
    };
    match escaped {
        Ok(e) => read(&e, false).unwrap_or(Sexp::Str(e)),
        Err(_) => Sexp::Str(s.to_owned()),
    }
}

/// org-babel-chomp: drop trailing whitespace characters in `set`.
pub fn chomp<'a>(s: &'a str, set: &str) -> &'a str {
    s.trim_end_matches(|c| set.contains(c))
}

pub const WS: &str = " \x0c\t\n\r\x0b";

/// org-trim.
pub fn trim(s: &str) -> &str {
    s.trim_matches(|c: char| " \t\n\r".contains(c))
}

/// org-remove-indentation: remove the common indentation of non-blank lines.
pub fn remove_indentation(s: &str) -> String {
    let ind = s
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| indentation(l))
        .min()
        .unwrap_or(0);
    if ind == 0 {
        return s.to_owned();
    }
    let mut out: Vec<String> = s.split('\n').map(|l| drop_columns(l, ind)).collect();
    if out.last().is_some_and(String::is_empty) && s.ends_with('\n') {
        out.pop();
        return out.join("\n") + "\n";
    }
    out.join("\n")
}

/// Columns of leading whitespace (tabs to 8).
pub fn indentation(l: &str) -> usize {
    let mut col = 0;
    for c in l.chars() {
        match c {
            ' ' => col += 1,
            '\t' => col = (col / 8 + 1) * 8,
            _ => break,
        }
    }
    col
}

fn drop_columns(l: &str, n: usize) -> String {
    let mut col = 0;
    for (i, c) in l.char_indices() {
        if col >= n {
            return format!("{}{}", " ".repeat(col - n), &l[i..]);
        }
        match c {
            ' ' => col += 1,
            '\t' => col = (col / 8 + 1) * 8,
            _ => return l[i..].to_owned(),
        }
    }
    String::new()
}

// ---- tables ----

/// A table row: cells, or None for a horizontal rule.
pub type Row = Option<Vec<String>>;

/// org-table-to-lisp on table lines.
pub fn table_to_lisp<S: AsRef<str>>(lines: &[S]) -> Vec<Row> {
    let mut out = vec![];
    for l in lines {
        let t = l.as_ref().trim_start_matches([' ', '\t']);
        let Some(rest) = t.strip_prefix('|') else {
            break;
        };
        if rest.starts_with('-') {
            out.push(None);
            continue;
        }
        let mut row = vec![];
        let mut s = rest;
        loop {
            s = s.trim_start_matches([' ', '\t']);
            if s.is_empty() {
                break;
            }
            let end = s.find('|').unwrap_or(s.len());
            row.push(s[..end].trim_end_matches([' ', '\t']).to_owned());
            s = if end < s.len() { &s[end + 1..] } else { "" };
        }
        out.push(Some(row));
    }
    out
}

fn is_table_number(s: &str) -> bool {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"^([<>]?[-+^.0-9]*[0-9][-+^.0-9eEdDx()%:]*|[<>]?[-+]?0[xX][[:xdigit:].]+|[<>]?[-+]?[0-9]+#[0-9a-zA-Z.]+|nan|[-+u]?inf)$").unwrap()
    })
    .is_match(s)
}

fn width(s: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(s)
}

/// org-table-align on parsed rows, with `indent` before each line.
pub fn align(rows: &[Row], indent: &str) -> Vec<String> {
    let cells: Vec<&Vec<String>> = rows.iter().flatten().collect();
    let ncols = cells.iter().map(|r| r.len()).max().unwrap_or(1).max(1);
    let mut widths = vec![1; ncols];
    let mut right = vec![false; ncols];
    for (i, (w, r)) in widths.iter_mut().zip(right.iter_mut()).enumerate() {
        let (mut numbers, mut non_empty) = (0usize, 0usize);
        let mut fixed: Option<bool> = None;
        for row in &cells {
            let c = row.get(i).map_or("", String::as_str);
            *w = (*w).max(width(c));
            if fixed.is_some() || c.is_empty() {
                continue;
            }
            if let Some(a) = c
                .strip_prefix('<')
                .and_then(|x| x.chars().next())
                .filter(|a| "lrc".contains(*a))
            {
                if c.ends_with('>') && c[2..c.len() - 1].bytes().all(|b| b.is_ascii_digit()) {
                    fixed = Some(a == 'r');
                    continue;
                }
            }
            non_empty += 1;
            if is_table_number(c) {
                numbers += 1;
            }
        }
        *r = fixed.unwrap_or(numbers as f64 >= 0.5 * non_empty as f64);
    }
    rows.iter()
        .map(|row| match row {
            None => format!(
                "{indent}|{}|",
                widths
                    .iter()
                    .map(|w| "-".repeat(w + 2))
                    .collect::<Vec<_>>()
                    .join("+")
            ),
            Some(r) => {
                let fields: Vec<String> = (0..ncols)
                    .map(|i| {
                        let c = r.get(i).map_or("", String::as_str);
                        let pad = " ".repeat(widths[i].saturating_sub(width(c)));
                        if right[i] {
                            format!(" {pad}{c} ")
                        } else {
                            format!(" {c}{pad} ")
                        }
                    })
                    .collect();
                format!("{indent}|{}|", fields.join("|"))
            }
        })
        .collect()
}

/// A Lisp table (list of rows and `hline`) as rows of printed cells.
pub fn lisp_to_rows(table: &Sexp) -> Vec<Row> {
    items(table)
        .unwrap_or(&[])
        .iter()
        .map(|r| {
            if is_hline(r) {
                None
            } else {
                Some(match items(r) {
                    Some(c) => c.iter().map(|x| princ(x).replace('\n', " ")).collect(),
                    None => vec![princ(r)],
                })
            }
        })
        .collect()
}

/// orgtbl-to-orgtbl + org-table-align: the table as aligned Org lines.
pub fn orgtbl(table: &Sexp) -> Vec<String> {
    let rows = lisp_to_rows(table);
    // Cells are parsed back from `| a |` text, as upstream does.
    let reparsed: Vec<Row> = rows
        .iter()
        .flat_map(|r| match r {
            None => vec![None],
            Some(c) => table_to_lisp(&[format!("| {} |", c.join(" | "))]),
        })
        .collect();
    align(&reparsed, "")
}

/// orgtbl-to-generic with `:sep`, `:fmt` and `:hline` (no special rows).
pub fn orgtbl_to_generic(
    table: &Sexp,
    sep: &str,
    fmt: &dyn Fn(&Sexp) -> String,
    hline: Option<&str>,
) -> String {
    let mut out = vec![];
    for r in items(table).unwrap_or(&[]) {
        if is_hline(r) {
            if let Some(h) = hline {
                out.push(h.to_owned());
            }
            continue;
        }
        match items(r) {
            Some(c) => out.push(
                c.iter()
                    .map(|x| {
                        if is_empty_cell(x) {
                            String::new()
                        } else {
                            fmt(x)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(sep),
            ),
            None => out.push(fmt(r)),
        }
    }
    out.join("\n")
}

fn is_empty_cell(x: &Sexp) -> bool {
    matches!(x, Sexp::Str(s) if s.is_empty())
}

/// orgtbl-to-csv cell quoting.
pub fn csv_field(s: &str) -> String {
    if s.contains(['"', ',', '\n']) || s.starts_with(' ') || s.ends_with(' ') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

/// A separator for [`convert_region`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sep {
    /// nil / `babel-auto`.
    Auto,
    Comma,
    Tab,
}

/// org-table-convert-region with `babel-auto` (or a forced separator),
/// then org-table-to-lisp of the aligned table: rows of cells, all the
/// same length.
pub fn convert_region(text: &str, sep: Sep) -> Result<Vec<Vec<String>>, String> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    let lines: Vec<&str> = text.split('\n').collect();
    let max = crate::org::options::int("org-table-convert-region-max-lines", 999);
    if lines.len() as i64 > max {
        return Err(format!(
            "Region is longer than `org-table-convert-region-max-lines' ({max}) lines; not converting"
        ));
    }
    let sep = match sep {
        Sep::Auto if lines.iter().all(|l| l.contains('\t')) => Sep::Tab,
        Sep::Auto if lines.iter().all(|l| l.contains(',')) => Sep::Comma,
        s => s,
    };
    let single = lines.len() == 1;
    let mut rows: Vec<Vec<String>> = lines
        .iter()
        .map(|l| match sep {
            Sep::Tab => l.split('\t').map(|c| c.trim().to_owned()).collect(),
            Sep::Comma => csv_split(l),
            // babel-auto with one line: no separator at all.
            Sep::Auto if single => vec![l.trim().to_owned()],
            Sep::Auto => l
                .split(|c| c == ' ' || c == '\t')
                .filter(|c| !c.is_empty())
                .map(str::to_owned)
                .collect(),
        })
        .map(|r: Vec<String>| r.into_iter().map(|c| c.replace('|', "")).collect())
        .collect();
    // The table text is re-read: a cell made of `-...` at a row start is a rule.
    let n = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
    for r in &mut rows {
        r.resize(n, String::new());
    }
    Ok(rows)
}

fn csv_split(l: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut chars = l.chars().peekable();
    let mut at_start = true;
    while let Some(c) = chars.next() {
        if at_start && c == '"' {
            at_start = false;
            while let Some(d) = chars.next() {
                if d == '"' {
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        cur.push('"');
                    } else {
                        break;
                    }
                } else {
                    cur.push(d);
                }
            }
            continue;
        }
        if c == ',' {
            out.push(cur.trim().to_owned());
            cur.clear();
            at_start = true;
            continue;
        }
        if at_start && (c == ' ' || c == '\t') {
            continue;
        }
        at_start = false;
        cur.push(c);
    }
    out.push(cur.trim().to_owned());
    out
}

/// org-babel-import-elisp-from-file on TEXT: a table, or a scalar for a
/// one-cell (or one-row) table; nil for empty text.
pub fn import_table(text: &str, sep: Sep) -> Result<Sexp, String> {
    if text.is_empty() {
        return Ok(Sexp::Nil);
    }
    let rows = convert_region(text, sep)?;
    let table: Vec<Sexp> = rows
        .iter()
        .map(|r| list(r.iter().map(|c| string_read(c)).collect()))
        .collect();
    Ok(match table.as_slice() {
        [row] => match items(row) {
            Some([x]) => x.clone(),
            Some(x) if x.len() >= 2 => list(table.clone()),
            _ => row.clone(),
        },
        _ => list(table),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_print_like_emacs() {
        for (f, s) in [
            (100.0, "100.0"),
            (1e20, "1e+20"),
            (1e15, "1e+15"),
            (123456789.0, "123456789.0"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (1.5, "1.5"),
            (12345678901234567.0, "12345678901234568.0"),
            (1.0 / 3.0, "0.3333333333333333"),
        ] {
            assert_eq!(float_str(f), s);
        }
    }

    #[test]
    fn org_babel_read_like_upstream() {
        for inhibit in [true, false] {
            assert_eq!(read("1", inhibit).unwrap(), Sexp::Int(1));
            assert_eq!(read("-1", inhibit).unwrap(), Sexp::Int(-1));
            assert_eq!(read("1.2", inhibit).unwrap(), Sexp::Float(1.2));
            assert_eq!(read(" 1 ", inhibit).unwrap(), Sexp::Int(1));
            assert_eq!(read(" 1\n", inhibit).unwrap(), Sexp::Int(1));
            assert_eq!(read("1foo", inhibit).unwrap(), str("1foo"));
            assert_eq!(read("", inhibit).unwrap(), str(""));
            assert_eq!(read(" \"foo\" ", inhibit).unwrap(), str("foo"));
            assert_eq!(
                read(" \"foo with\\\" inside\" ", inhibit).unwrap(),
                str("foo with\" inside")
            );
            assert_eq!(
                read("\"foo\"(\"bar\"", inhibit).unwrap(),
                str("\"foo\"(\"bar\"")
            );
            assert_eq!(
                read("\"foo\"\"bar\"", inhibit).unwrap(),
                str("\"foo\"\"bar\"")
            );
        }
        assert_eq!(read("'(1 2)", false).unwrap(), sexp::read("(1 2)").unwrap());
        assert_eq!(read("'(1 2)", true).unwrap(), str("'(1 2)"));
        assert_eq!(
            read("[1 2 (foo)]", false).unwrap().to_string(),
            "[1 2 (foo)]"
        );
        assert_eq!(string_to_number("1e3"), Some(Sexp::Float(1000.0)));
        assert_eq!(string_to_number("010"), Some(Sexp::Int(10)));
        assert_eq!(string_to_number("1 2"), None);
    }

    #[test]
    fn script_escape_like_upstream() {
        assert_eq!(prin1(&script_escape("[1, 2, 3]", false)), "(1 2 3)");
        assert_eq!(
            prin1(&script_escape("[[1, 'a'], [2, \"b\"]]", false)),
            "((1 \"a\") (2 \"b\"))"
        );
        assert_eq!(prin1(&script_escape("'hi'", false)), "\"hi\"");
        assert_eq!(prin1(&script_escape("{1, 2}", false)), "(1 2)");
        assert_eq!(prin1(&script_escape("plain", false)), "\"plain\"");
        assert_eq!(prin1(&script_escape("['a\\'b']", false)), "(\"a'b\")");
    }

    #[test]
    fn tables_align_and_import() {
        let t = sexp::read("((1 \"a\") hline (10 \"bb\"))").unwrap();
        let t = cons(cons(str("x"), list(vec![str("y")])), t);
        assert_eq!(
            orgtbl(&t),
            vec!["|  x | y  |", "|  1 | a  |", "|----+----|", "| 10 | bb |"]
        );
        assert_eq!(
            prin1(&import_table("line 1\nline 2\n", Sep::Auto).unwrap()),
            "((\"line\" 1) (\"line\" 2))"
        );
        assert_eq!(
            prin1(&import_table("1\t2\n3\t4", Sep::Auto).unwrap()),
            "((1 2) (3 4))"
        );
        assert_eq!(
            prin1(&import_table("hello world", Sep::Auto).unwrap()),
            "\"hello world\""
        );
        assert_eq!(
            prin1(&import_table("a,\"b,c\"\n1,2", Sep::Auto).unwrap()),
            "((\"a\" \"b,c\") (1 2))"
        );
        assert_eq!(import_table("", Sep::Auto).unwrap(), Sexp::Nil);
        assert_eq!(remove_indentation("  a\n    b\n"), "a\n  b\n");
    }
}
