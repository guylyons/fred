//! `:set`, `:setlocal` and `:setglobal` arguments (`:h :set-args`).

use super::{OPTIONS, Opt, Options, Value};

/// Which value a command reads and writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// `:set`: the local value (if any) and the global one.
    Both,
    /// `:setlocal`.
    Local,
    /// `:setglobal`.
    Global,
}

/// Run `:set`-style `args` at `level`; returns what to show (`:set ts?`).
/// Every argument before an error has taken effect, as in Neovim.
pub fn set(opts: &mut Options, args: &str, level: Level) -> Result<String, String> {
    let mut shown = vec![];
    let words = split(args);
    if words.is_empty() || words == ["all"] {
        // `:set` lists what differs from its default; `:set all`, everything.
        let all = !words.is_empty();
        for (i, d) in OPTIONS.iter().enumerate() {
            let o = Opt::ALL[i];
            if all || opts.get_at(o, level) != d.default.into() {
                shown.push(show(opts, o, level));
            }
        }
        return Ok(shown.join("  "));
    }
    for word in words {
        if let Some(text) = one(opts, &word, level)? {
            shown.push(text);
        }
    }
    Ok(shown.join("  "))
}

/// One argument; Some when it asks to see a value.
fn one(opts: &mut Options, arg: &str, level: Level) -> Result<Option<String>, String> {
    if arg == "all&" {
        for (i, d) in OPTIONS.iter().enumerate() {
            opts.put(Opt::ALL[i], d.default.into(), level)?;
        }
        return Ok(None);
    }
    let name_end = arg
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(arg.len());
    let (name, op) = arg.split_at(name_end);
    let (o, prefix) = match Opt::find(name) {
        Some(o) => (o, ""),
        None => {
            let found = ["no", "inv"]
                .into_iter()
                .find_map(|p| Some((Opt::find(name.strip_prefix(p)?)?, p)));
            match found {
                Some((o, p)) if matches!(o.def().default, super::DefaultValue::Bool(_)) => (o, p),
                _ => return Err(format!("E518: Unknown option: {name}")),
            }
        }
    };
    let cur = opts.get_at(o, level);
    let invalid = || Err(format!("E474: Invalid argument: {arg}"));
    let new = match (op, &cur) {
        ("?", _) if prefix.is_empty() => return Ok(Some(show(opts, o, level))),
        ("&" | "&vim", _) if prefix.is_empty() => o.def().default.into(),
        ("<", _) if prefix.is_empty() && level == Level::Local => opts.get_at(o, Level::Global),
        ("" | "!", Value::Bool(b)) => Value::Bool(match (prefix, op) {
            ("inv", "") | ("", "!") => !b,
            ("no", "") => false,
            ("", "") => true,
            _ => return invalid(),
        }),
        ("", _) if prefix.is_empty() => return Ok(Some(show(opts, o, level))),
        _ if !prefix.is_empty() || matches!(cur, Value::Bool(_)) => return invalid(),
        _ => {
            let Some((how, value)) = ["+=", "-=", "^=", "=", ":"]
                .into_iter()
                .find_map(|p| Some((p, op.strip_prefix(p)?)))
            else {
                return invalid();
            };
            change(o, &cur, how, value, arg)?
        }
    };
    opts.put(o, new, level)?;
    Ok(None)
}

/// `cur` changed by `=`, `+=`, `-=` or `^=` with `value`.
fn change(o: Opt, cur: &Value, how: &str, value: &str, arg: &str) -> Result<Value, String> {
    Ok(match cur {
        Value::Num(n) => {
            let v = number(value).ok_or_else(|| format!("E521: Number required after =: {arg}"))?;
            Value::Num(match how {
                "+=" => n + v,
                "-=" => n - v,
                "^=" => n * v,
                _ => v,
            })
        }
        Value::Str(s) if o.def().list => {
            let mut items: Vec<&str> = s.split(',').filter(|i| !i.is_empty()).collect();
            let present = items.contains(&value);
            match how {
                "+=" if !present => items.push(value),
                "^=" if !present => items.insert(0, value),
                "-=" => items.retain(|i| *i != value),
                "=" | ":" => items = vec![value].into_iter().filter(|v| !v.is_empty()).collect(),
                _ => {}
            }
            Value::Str(items.join(","))
        }
        Value::Str(s) => Value::Str(match how {
            "+=" => format!("{s}{value}"),
            "^=" => format!("{value}{s}"),
            "-=" => s.replacen(value, "", 1),
            _ => value.to_string(),
        }),
        Value::Bool(_) => return Err(format!("E474: Invalid argument: {arg}")),
    })
}

/// Decimal, or hex after `0x`.
fn number(s: &str) -> Option<i64> {
    let (neg, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let n = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => i64::from_str_radix(hex, 16).ok()?,
        None => s.parse().ok()?,
    };
    Some(if neg { -n } else { n })
}

/// How `:set` shows an option: `wrap`, `nowrap`, `tabstop=8`.
fn show(opts: &Options, o: Opt, level: Level) -> String {
    let name = o.def().name;
    match opts.get_at(o, level) {
        Value::Bool(true) => name.to_string(),
        Value::Bool(false) => format!("no{name}"),
        Value::Num(n) => format!("{name}={n}"),
        Value::Str(s) => format!("{name}={s}"),
    }
}

/// Arguments split at white space; `\ ` and `\\` escape.
fn split(args: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut chars = args.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => cur.extend(chars.next()),
            c if c.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}
