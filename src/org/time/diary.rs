//! Diary sexp timestamps `<%%(...)>` evaluated natively: the diary-lib
//! entry functions (diary-date, diary-block, diary-float,
//! diary-anniversary, diary-cyclic, diary-offset), Org's ISO-order
//! wrappers (org-date, org-block, org-anniversary, org-cyclic,
//! org-class), org-calendar-holiday, and `and`/`or`/`not`.
//!
//! Holidays: the default `calendar-holidays` subsets that need no other
//! calendars (holiday-general-holidays, Easter-based and Christmas from
//! holiday-christian-holidays). Hebrew, Islamic, Bahá'í, Chinese and
//! solar holidays are not computed.

use super::civil::{Tm, month_days};
use super::sexp::{self, Sexp};

/// An evaluated value.
#[derive(Clone, Debug, PartialEq)]
enum V {
    Nil,
    T,
    Int(i64),
    Str(String),
    List(Vec<V>),
    /// `(cons mark entry)` of the diary functions.
    Entry(String),
}

impl V {
    fn truthy(&self) -> bool {
        *self != V::Nil
    }
}

struct Env<'a> {
    entry: &'a str,
    date: Tm,
}

/// org-diary-sexp-entry: the entry texts when `sexp` (the text of
/// `(...)`) applies on `date`, None when it does not (or is bad).
pub fn entry(sexp: &str, entry: &str, date: Tm) -> Option<Vec<String>> {
    let form = sexp::read(sexp).ok()?;
    let env = Env {
        entry,
        date: date.norm(),
    };
    match eval(&form, &env).ok()? {
        V::Nil => None,
        V::Str(s) => Some(s.split("; ").map(str::to_owned).collect()),
        V::Entry(s) => Some(vec![s]),
        V::List(l) if l.iter().all(|x| matches!(x, V::Str(_))) && !l.is_empty() => Some(
            l.into_iter()
                .map(|x| if let V::Str(s) = x { s } else { String::new() })
                .collect(),
        ),
        _ => Some(vec![entry.to_owned()]),
    }
}

fn data(s: &Sexp) -> V {
    match s {
        Sexp::Nil => V::Nil,
        Sexp::T => V::T,
        Sexp::Int(i) => V::Int(*i),
        Sexp::Float(f) => V::Int(*f as i64),
        Sexp::Str(s) => V::Str(s.clone()),
        Sexp::Sym(s) if s == "t" => V::T,
        Sexp::Sym(s) if s == "nil" => V::Nil,
        Sexp::Sym(s) => V::Str(s.clone()),
        Sexp::List(v) | Sexp::Vector(v) if v.is_empty() => V::Nil,
        Sexp::List(v) | Sexp::Vector(v) => V::List(v.iter().map(data).collect()),
        Sexp::Dotted(v, _) => V::List(v.iter().map(data).collect()),
    }
}

fn eval(form: &Sexp, env: &Env) -> Result<V, String> {
    match form {
        Sexp::Sym(s) => match s.as_str() {
            "t" => Ok(V::T),
            "nil" => Ok(V::Nil),
            "entry" => Ok(V::Str(env.entry.to_owned())),
            "date" => Ok(V::List(vec![
                V::Int(env.date.month),
                V::Int(env.date.day),
                V::Int(env.date.year),
            ])),
            other => Err(format!("Symbol's value as variable is void: {other}")),
        },
        Sexp::List(v) if !v.is_empty() => {
            let f = v[0].sym().ok_or("Invalid function")?;
            let rest = &v[1..];
            match f {
                "quote" => return Ok(rest.first().map_or(V::Nil, data)),
                "and" => {
                    let mut last = V::T;
                    for x in rest {
                        last = eval(x, env)?;
                        if !last.truthy() {
                            return Ok(V::Nil);
                        }
                    }
                    return Ok(last);
                }
                "or" => {
                    for x in rest {
                        let r = eval(x, env)?;
                        if r.truthy() {
                            return Ok(r);
                        }
                    }
                    return Ok(V::Nil);
                }
                "diary-offset" => {
                    let days = rest.get(1).map(|d| eval(d, env)).transpose()?;
                    let Some(V::Int(days)) = days else {
                        return Err("Days must be an integer".into());
                    };
                    let inner = Env {
                        entry: env.entry,
                        date: env.date.add(super::civil::Unit::Day, -days),
                    };
                    let form = rest.first().ok_or("diary-offset: no sexp")?;
                    let form = match form {
                        Sexp::List(q) if q.len() == 2 && q[0].sym() == Some("quote") => &q[1],
                        f => f,
                    };
                    return eval(form, &inner);
                }
                _ => {}
            }
            let args = rest
                .iter()
                .map(|x| eval(x, env))
                .collect::<Result<Vec<_>, _>>()?;
            call(f, &args, env)
        }
        other => Ok(data(other)),
    }
}

fn int(v: Option<&V>) -> Result<i64, String> {
    match v {
        Some(V::Int(i)) => Ok(*i),
        _ => Err("Wrong type argument: integerp".into()),
    }
}

/// diary-make-date: (month day year) from arguments in
/// `calendar-date-style` order.
fn make_date<'a>(a: &'a V, b: &'a V, c: &'a V, iso: bool) -> (&'a V, &'a V, &'a V) {
    let style = if iso {
        "iso".to_owned()
    } else {
        super::options::string("calendar-date-style", "american")
    };
    match style.as_str() {
        "iso" => (b, c, a),
        "european" => (b, a, c),
        _ => (a, b, c),
    }
}

fn matches(spec: &V, x: i64) -> bool {
    match spec {
        V::T => true,
        V::Int(i) => *i == x,
        V::List(l) => l.contains(&V::Int(x)),
        _ => false,
    }
}

fn ordinal(n: i64) -> &'static str {
    if [11, 12, 13].contains(&(n % 100)) || n % 10 > 3 {
        "th"
    } else {
        ["th", "st", "nd", "rd"][(n % 10) as usize]
    }
}

/// `(format entry n suffix)` for %d / %s.
fn format_entry(entry: &str, n: i64) -> String {
    let mut out = String::new();
    let mut args = vec![n.to_string(), ordinal(n).to_owned()].into_iter();
    let mut it = entry.chars();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('d' | 's') => out.push_str(&args.next().unwrap_or_default()),
            Some('%') => out.push('%'),
            Some(o) => {
                out.push('%');
                out.push(o);
            }
            None => out.push('%'),
        }
    }
    out
}

/// calendar-nth-named-absday.
pub fn nth_named_absday(n: i64, dayname: i64, month: i64, year: i64, day: Option<i64>) -> i64 {
    let on_or_before = |d: i64| d - (d - dayname).rem_euclid(7);
    if n > 0 {
        7 * (n - 1) + on_or_before(6 + Tm::date(year, month, day.unwrap_or(1)).absolute())
    } else {
        7 * (n + 1)
            + on_or_before(
                Tm::date(year, month, day.unwrap_or_else(|| month_days(year, month))).absolute(),
            )
    }
}

fn call(f: &str, a: &[V], env: &Env) -> Result<V, String> {
    let date = env.date;
    let arg = |i: usize| a.get(i).unwrap_or(&V::Nil);
    let entry = || V::Entry(env.entry.to_owned());
    Ok(match f {
        "not" | "null" => {
            if arg(0).truthy() {
                V::Nil
            } else {
                V::T
            }
        }
        "diary-date" | "org-date" => {
            let (m, d, y) = make_date(arg(0), arg(1), arg(2), f == "org-date");
            if matches(d, date.day) && matches(m, date.month) && matches(y, date.year) {
                entry()
            } else {
                V::Nil
            }
        }
        "diary-block" | "org-block" => {
            let iso = f == "org-block";
            let (m1, d1, y1) = make_date(arg(0), arg(1), arg(2), iso);
            let (m2, d2, y2) = make_date(arg(3), arg(4), arg(5), iso);
            let d1 = Tm::date(int(Some(y1))?, int(Some(m1))?, int(Some(d1))?).absolute();
            let d2 = Tm::date(int(Some(y2))?, int(Some(m2))?, int(Some(d2))?).absolute();
            let d = date.absolute();
            if d1 <= d && d <= d2 { entry() } else { V::Nil }
        }
        "diary-anniversary" | "org-anniversary" => {
            let (m, d, y) = make_date(arg(0), arg(1), arg(2), f == "org-anniversary");
            let (mut mm, mut dd) = (int(Some(m))?, int(Some(d))?);
            let diff = match y {
                V::Int(y) => date.year - y,
                _ => 100,
            };
            if mm == 2 && dd == 29 && !super::civil::leap(date.year) {
                mm = 3;
                dd = 1;
            }
            if diff > 0 && mm == date.month && dd == date.day {
                V::Entry(format_entry(env.entry, diff))
            } else {
                V::Nil
            }
        }
        "diary-cyclic" | "org-cyclic" => {
            let n = int(a.first())?;
            if n <= 0 {
                return Err("Day count must be positive".into());
            }
            let (m, d, y) = make_date(arg(1), arg(2), arg(3), f == "org-cyclic");
            let diff =
                date.absolute() - Tm::date(int(Some(y))?, int(Some(m))?, int(Some(d))?).absolute();
            if diff >= 0 && diff % n == 0 {
                V::Entry(format_entry(env.entry, diff / n))
            } else {
                V::Nil
            }
        }
        "diary-float" => {
            let month = arg(0);
            let dayname = int(a.get(1))?;
            let n = int(a.get(2))?;
            let day = match arg(3) {
                V::Int(d) => Some(*d),
                _ => None,
            };
            if dayname != date.weekday() {
                return Ok(V::Nil);
            }
            let limit = nth_named_absday(-n, dayname, date.month, date.year, Some(date.day));
            let (last_abs, first_abs) = if n > 0 {
                (limit, limit - 6)
            } else {
                (limit + 6, limit)
            };
            let (last, first) = (Tm::from_absolute(last_abs), Tm::from_absolute(first_abs));
            let base_day = |m: i64, y: i64| day.unwrap_or(if n > 0 { 1 } else { month_days(y, m) });
            let ok = (first.month == last.month
                && matches(month, first.month)
                && (first.day..=last.day).contains(&base_day(first.month, first.year)))
                || ((first.year < last.year
                    || (first.year == last.year && first.month < last.month))
                    && ((matches(month, first.month)
                        && first.day <= base_day(first.month, first.year))
                        || (matches(month, last.month)
                            && base_day(last.month, last.year) <= last.day)));
            if ok { entry() } else { V::Nil }
        }
        "org-class" => {
            let d1 = Tm::date(int(a.first())?, int(a.get(1))?, int(a.get(2))?).absolute();
            let d2 = Tm::date(int(a.get(3))?, int(a.get(4))?, int(a.get(5))?).absolute();
            let dayname = int(a.get(6))?;
            let skip = &a[7.min(a.len())..];
            let d = date.absolute();
            let hols = if skip.is_empty() {
                vec![]
            } else {
                holidays(date)
            };
            let week = super::civil::iso_week(date).0;
            let skipped = skip.contains(&V::Int(week))
                || (!hols.is_empty() && skip.iter().any(|s| *s == V::Str("holidays".into())))
                || hols.iter().any(|h| skip.contains(&V::Str(h.clone())));
            if d1 <= d && d <= d2 && date.weekday() == dayname && !skipped {
                V::Str(env.entry.to_owned())
            } else {
                V::Nil
            }
        }
        "org-calendar-holiday" | "calendar-check-holidays" => {
            let h = holidays(date);
            if h.is_empty() {
                V::Nil
            } else if f == "org-calendar-holiday" {
                V::Str(h.join("; "))
            } else {
                V::List(h.into_iter().map(V::Str).collect())
            }
        }
        "diary-ordinal-suffix" => V::Str(ordinal(int(a.first())?).to_owned()),
        "calendar-day-of-week" => V::Int(date.weekday()),
        "calendar-extract-month" => V::Int(date.month),
        "calendar-extract-day" => V::Int(date.day),
        "calendar-extract-year" => V::Int(date.year),
        "=" | "eq" | "equal" => {
            if a.windows(2).all(|w| w[0] == w[1]) {
                V::T
            } else {
                V::Nil
            }
        }
        "memq" | "member" => match arg(1) {
            V::List(l) if l.contains(arg(0)) => V::T,
            _ => V::Nil,
        },
        _ => return Err(format!("Symbol's function definition is void: {f}")),
    })
}

/// Easter Sunday's absolute day (holiday-easter-etc-abs).
pub fn easter(y: i64) -> i64 {
    let century = 1 + y / 100;
    let shifted =
        (14 + 11 * (y % 19) - (3 * century) / 4 + (5 + 8 * century) / 25 + 30 * century) % 30;
    let adjusted = if shifted == 0 || (shifted == 1 && 10 < y % 19) {
        shifted + 1
    } else {
        shifted
    };
    let paschal = Tm::date(y, 4, 19).absolute() - adjusted;
    let d = paschal + 7;
    d - d.rem_euclid(7)
}

/// The holidays on `date` (calendar-check-holidays for the general and
/// Christian lists).
pub fn holidays(date: Tm) -> Vec<String> {
    let y = date.year;
    let abs = date.absolute();
    let fixed = |m: i64, d: i64| Tm::date(y, m, d).absolute();
    let float = |m: i64, wd: i64, n: i64| nth_named_absday(n, wd, m, y, None);
    let list: [(i64, &str); 20] = [
        (fixed(1, 1), "New Year's Day"),
        (float(1, 1, 3), "Martin Luther King Day"),
        (fixed(2, 2), "Groundhog Day"),
        (fixed(2, 14), "Valentine's Day"),
        (float(2, 1, 3), "President's Day"),
        (fixed(3, 17), "St. Patrick's Day"),
        (fixed(4, 1), "April Fools' Day"),
        (float(5, 0, 2), "Mother's Day"),
        (float(5, 1, -1), "Memorial Day"),
        (fixed(6, 14), "Flag Day"),
        (float(6, 0, 3), "Father's Day"),
        (fixed(7, 4), "Independence Day"),
        (float(9, 1, 1), "Labor Day"),
        (float(10, 1, 2), "Columbus Day"),
        (fixed(10, 31), "Halloween"),
        (fixed(11, 11), "Veteran's Day"),
        (float(11, 4, 4), "Thanksgiving"),
        (easter(y) - 2, "Good Friday"),
        (easter(y), "Easter Sunday"),
        (fixed(12, 25), "Christmas"),
    ];
    list.iter()
        .filter(|(d, _)| *d == abs)
        .map(|(_, n)| (*n).to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(s: &str, y: i64, m: i64, d: i64) -> Option<Vec<String>> {
        entry(s, "E %d%s", Tm::date(y, m, d))
    }

    #[test]
    fn diary_functions() {
        // Thanksgiving 2026: 4th Thursday of November is the 26th.
        assert!(on("(diary-float 11 4 4)", 2026, 11, 26).is_some());
        assert!(on("(diary-float 11 4 4)", 2026, 11, 19).is_none());
        // Last Monday of May.
        assert!(on("(diary-float 5 1 -1)", 2026, 5, 25).is_some());
        assert!(on("(diary-float t 1 1)", 2026, 10, 5).is_some());
        assert_eq!(
            on("(diary-anniversary 10 4 2000)", 2026, 10, 4),
            Some(vec!["E 26th".into()])
        );
        assert_eq!(
            on("(org-anniversary 2000 10 4)", 2026, 10, 4),
            Some(vec!["E 26th".into()])
        );
        assert_eq!(
            on("(diary-cyclic 7 10 4 2026)", 2026, 10, 18),
            Some(vec!["E 2nd".into()])
        );
        assert!(on("(org-block 2026 10 1 2026 10 10)", 2026, 10, 10).is_some());
        assert!(on("(org-block 2026 10 1 2026 10 10)", 2026, 10, 11).is_none());
        assert!(on("(diary-date '(10 12) t t)", 2026, 11, 4).is_none());
        assert!(on("(diary-date 10 t t)", 2026, 10, 9).is_some());
        assert!(on("(org-date 2026 10 t)", 2026, 10, 9).is_some());
        assert!(
            on(
                "(and (diary-float t 5 1) (not (diary-date 1 t t)))",
                2026,
                10,
                2
            )
            .is_some()
        );
        assert_eq!(
            on("(org-calendar-holiday)", 2026, 4, 5),
            Some(vec!["Easter Sunday".into()])
        );
        assert_eq!(
            on("(org-calendar-holiday)", 2026, 11, 26),
            Some(vec!["Thanksgiving".into()])
        );
        assert!(
            on("(org-class 2026 9 1 2026 12 20 1 41)", 2026, 10, 5).is_none(),
            "ISO week 41 skipped"
        );
        assert!(on("(org-class 2026 9 1 2026 12 20 1)", 2026, 10, 5).is_some());
        assert!(on("(diary-offset '(diary-date 10 4 2026) 1)", 2026, 10, 5).is_some());
        assert!(on("(bogus)", 2026, 10, 5).is_none());
    }
}
