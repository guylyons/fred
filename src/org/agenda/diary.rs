//! org-get-entries-from-diary: the Emacs diary for one day (holidays
//! and the entries of `diary-file`), as agenda items.
//!
//! Adaptation: there is no Emacs calendar, so diary-lib's
//! diary-list-entries is replaced by a reader of the common entry forms
//! (`calendar-date-style` american): `MONTH/DAY[/YEAR]`, `Month DAY[,
//! YEAR]` (full or abbreviated names, `*` wildcards), day names, and
//! `%%(sexp)` entries evaluated by time::diary. Holidays are those of
//! time::diary::holidays (diary-show-holidays-flag).

use super::collect::Ctx;
use super::format::{self, In};
use super::{Dotime, Face, Item, Loc, opt, opt_bool};
use crate::org::time::civil::{self, Tm};
use std::path::PathBuf;

/// The diary file (`diary-file`, default ~/diary or ~/.emacs.d/diary).
pub fn file() -> PathBuf {
    if let Some(f) = opt("diary-file").and_then(|v| v.str().map(str::to_owned)) {
        return crate::org::options::expand(&f);
    }
    let home = crate::org::options::expand("~/diary");
    if home.exists() {
        home
    } else {
        crate::org::options::expand("~/.emacs.d/diary")
    }
}

fn month_of(name: &str) -> Option<i64> {
    let n = name.trim_end_matches('.').to_lowercase();
    if n.len() < 3 {
        return None;
    }
    civil::MONTH_NAMES
        .iter()
        .position(|m| m.to_lowercase().starts_with(&n) && (n.len() == 3 || m.to_lowercase() == n))
        .map(|i| i as i64 + 1)
}

fn day_of_week(name: &str) -> Option<i64> {
    let n = name.trim_end_matches('.').to_lowercase();
    if n.len() < 3 {
        return None;
    }
    civil::DAY_NAMES
        .iter()
        .position(|d| d.to_lowercase().starts_with(&n) && (n.len() == 3 || d.to_lowercase() == n))
        .map(|i| i as i64)
}

fn num_or_star(s: &str, want: i64) -> Option<bool> {
    if s == "*" {
        return Some(true);
    }
    s.parse::<i64>().ok().map(|n| n == want)
}

/// The text of `entry` (its first line, the date removed) when it applies on `d`.
fn applies(first: &str, d: Tm) -> Option<String> {
    let t = first.strip_prefix('&').unwrap_or(first);
    if let Some(rest) = t.strip_prefix("%%") {
        let end = sexp_end(rest)?;
        let entry = rest[end..].trim().to_owned();
        return crate::org::time::diary::entry(&rest[..end], &entry, d).map(|v| v.join("; "));
    }
    // MONTH/DAY[/YEAR]
    if let Some(c) = re!(r"^([0-9]+|\*)/([0-9]+|\*)(?:/([0-9]+|\*))?(.*)$").captures(t) {
        let y_ok = c.get(3).is_none_or(|y| {
            num_or_star(y.as_str(), d.year).unwrap_or(false)
                || y.as_str()
                    .parse::<i64>()
                    .is_ok_and(|n| n < 100 && n + 1900 == d.year || n + 2000 == d.year)
        });
        return (num_or_star(&c[1], d.month)? && num_or_star(&c[2], d.day)? && y_ok)
            .then(|| c[4].trim().to_owned());
    }
    // Month DAY[, YEAR]
    if let Some(c) = re!(r"^([A-Za-z]+\.?|\*) +([0-9]+|\*)(?:, *([0-9]+|\*))?(.*)$").captures(t)
        && (c[1].starts_with('*') || month_of(&c[1]).is_some())
    {
        let m_ok = &c[1] == "*" || month_of(&c[1]) == Some(d.month);
        let y_ok = c
            .get(3)
            .is_none_or(|y| num_or_star(y.as_str(), d.year).unwrap_or(false));
        return (m_ok && num_or_star(&c[2], d.day)? && y_ok).then(|| c[4].trim().to_owned());
    }
    // Day name.
    let word: String = t
        .chars()
        .take_while(|c| c.is_alphabetic() || *c == '.')
        .collect();
    if let Some(w) = day_of_week(&word) {
        return (w == d.weekday()).then(|| t[word.len()..].trim().to_owned());
    }
    None
}

fn sexp_end(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// The diary entries of absolute day `date` as agenda items.
pub fn entries(ctx: &Ctx, date: i64) -> Vec<Item> {
    let d = Tm::from_absolute(date);
    let mut texts: Vec<(String, Option<Loc>)> = vec![];
    if opt("diary-show-holidays-flag").is_none_or(|v| v.truthy())
        && opt_bool("calendar-holidays-enabled", true)
    {
        for h in crate::org::time::diary::holidays(d) {
            texts.push((h, None));
        }
    }
    let path = file();
    if let Ok(text) = std::fs::read_to_string(&path) {
        let lines: Vec<&str> = text.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            let first = lines[i];
            let start = i;
            i += 1;
            if first.trim().is_empty() || first.starts_with([' ', '\t']) {
                continue;
            }
            let mut cont = vec![];
            while i < lines.len() && lines[i].starts_with([' ', '\t']) {
                cont.push(lines[i].trim());
                i += 1;
            }
            if let Some(t) = applies(first, d) {
                let mut parts = vec![t];
                parts.extend(cont.iter().map(|s| (*s).to_owned()));
                let parts: Vec<String> = parts.into_iter().filter(|p| !p.is_empty()).collect();
                texts.push((
                    parts.join("; "),
                    Some(Loc {
                        path: path.clone(),
                        line: start,
                        text: first.to_owned(),
                        col: 0,
                    }),
                ));
            }
        }
    }
    texts
        .into_iter()
        .map(|(t, loc)| {
            let mut it = format::item(
                &ctx.prefix,
                In {
                    extra: Some(String::new()),
                    txt: t.clone(),
                    category: Some("Diary".into()),
                    dotime: Dotime::Yes,
                    ..In::default()
                },
            );
            it.head = t;
            it.marker = loc;
            it.diary_link = true;
            it.kind = "diary".into();
            it.date = Some(date);
            it.face = Face::Diary;
            it.fmt = ctx.fmt;
            it
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_forms() {
        let d = Tm::date(2026, 10, 12); // a Monday
        assert_eq!(
            applies("Oct 12, 2026 Dentist", d).as_deref(),
            Some("Dentist")
        );
        assert_eq!(applies("October 12 Yearly", d).as_deref(), Some("Yearly"));
        assert_eq!(applies("10/12/2026 Slash", d).as_deref(), Some("Slash"));
        assert_eq!(applies("*/12 Monthly", d).as_deref(), Some("Monthly"));
        assert_eq!(applies("Monday Weekly", d).as_deref(), Some("Weekly"));
        assert_eq!(applies("Tuesday Weekly", d), None);
        assert_eq!(
            applies("%%(diary-float 10 1 2) Float", d).as_deref(),
            Some("Float")
        );
        assert_eq!(applies("&Oct 13 x", d), None);
    }
}
