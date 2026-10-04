//! org-duration.el: durations as `H:MM`, `H:MM:SS`, unit strings
//! (`3d 2h 10min`) or both (`1d 3:20`), to and from minutes, under
//! org-duration-units and org-duration-format.

use super::sexp::{self, Sexp};
use std::cell::RefCell;

/// org-duration-canonical-units.
pub const CANONICAL: [(&str, f64); 3] = [("min", 1.0), ("h", 60.0), ("d", 1440.0)];

/// org-duration-units (unit, minutes), from the option or the default.
pub fn units() -> Vec<(String, f64)> {
    let default = || {
        [
            ("min", 1.0),
            ("h", 60.0),
            ("d", 1440.0),
            ("w", 10080.0),
            ("m", 43200.0),
            ("y", 525_960.0),
        ]
        .iter()
        .map(|(u, m)| ((*u).to_owned(), *m))
        .collect()
    };
    let Some(v) = sexp::option("org-duration-units") else {
        return default();
    };
    let Some(l) = v.list() else { return default() };
    l.iter()
        .filter_map(|e| {
            let u = e.car()?.str()?.to_owned();
            let m = match e.cdr() {
                Sexp::Int(i) => i as f64,
                Sexp::Float(f) => f,
                Sexp::List(l) => match l.first() {
                    Some(Sexp::Int(i)) => *i as f64,
                    Some(Sexp::Float(f)) => *f,
                    _ => return None,
                },
                _ => return None,
            };
            Some((u, m))
        })
        .collect()
}

/// org-duration--modifier.
pub fn modifier(unit: &str, canonical: bool) -> Result<f64, String> {
    let found = if canonical {
        CANONICAL.iter().find(|(u, _)| *u == unit).map(|x| x.1)
    } else {
        units().into_iter().find(|(u, _)| u == unit).map(|x| x.1)
    };
    found.ok_or_else(|| format!("Unknown unit: \"{unit}\""))
}

struct Regexps {
    key: Vec<(String, f64)>,
    unit: regex::Regex,
    full: regex::Regex,
    mixed: regex::Regex,
}

thread_local! {
    static RES: RefCell<Option<std::rc::Rc<Regexps>>> = const { RefCell::new(None) };
}

/// org-duration-set-regexps: the unit regexps for the current
/// org-duration-units (rebuilt whenever the option changes).
pub fn set_regexps() {
    RES.with(|r| *r.borrow_mut() = None);
    regexps();
}

fn regexps() -> std::rc::Rc<Regexps> {
    let key = units();
    if let Some(r) = RES.with(|r| r.borrow().clone())
        && r.key == key
    {
        return r;
    }
    let mut names: Vec<String> = CANONICAL
        .iter()
        .map(|(u, _)| (*u).to_owned())
        .chain(key.iter().map(|(u, _)| u.clone()))
        .collect();
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    names.dedup();
    let alt = names
        .iter()
        .map(|n| regex::escape(n))
        .collect::<Vec<_>>()
        .join("|");
    let unit = format!(r"([0-9]+(?:\.[0-9]*)?)[ \t]*({alt})");
    let r = std::rc::Rc::new(Regexps {
        key,
        full: regex::Regex::new(&format!(r"^(?:[ \t]*{unit})+[ \t]*$")).unwrap(),
        mixed: regex::Regex::new(&format!(
            r"^((?:[ \t]*{unit})+)[ \t]*([0-9]+(?::[0-9][0-9]){{1,2}})[ \t]*$"
        ))
        .unwrap(),
        unit: regex::Regex::new(&unit).unwrap(),
    });
    RES.with(|c| *c.borrow_mut() = Some(r.clone()));
    r
}

fn hmm_re() -> &'static regex::Regex {
    re!(r"^[ \t]*[0-9]+(?::[0-9]{2}){1,2}[ \t]*$")
}

fn hmmss_re() -> &'static regex::Regex {
    re!(r"^[ \t]*[0-9]+(?::[0-9]{2}){2}[ \t]*$")
}

/// org-duration-p.
pub fn is_duration(s: &str) -> bool {
    let r = regexps();
    r.full.is_match(s) || r.mixed.is_match(s) || hmm_re().is_match(s)
}

/// org-duration-to-minutes.
pub fn to_minutes(d: &str, canonical: bool) -> Result<f64, String> {
    if d.is_empty() {
        return Ok(0.0);
    }
    let r = regexps();
    if hmm_re().is_match(d) {
        let p: Vec<f64> = d
            .split(':')
            .map(|x| x.trim().parse().unwrap_or(0.0))
            .collect();
        return Ok(p.get(2).copied().unwrap_or(0.0) / 60.0 + p[1] + 60.0 * p[0]);
    }
    if r.full.is_match(d) {
        let mut m = 0.0;
        for c in r.unit.captures_iter(d) {
            let v: f64 = c[1].parse().unwrap_or(0.0);
            m += v * modifier(&c[2], canonical)?;
        }
        return Ok(m);
    }
    if let Some(c) = r.mixed.captures(d) {
        let units_part = c.get(1).unwrap().as_str();
        let hms = c.get(c.len() - 1).unwrap().as_str();
        return Ok(to_minutes(units_part, false)? + to_minutes(hms, false)?);
    }
    if re!(r"^[0-9]+(\.[0-9]*)?$").is_match(d) {
        return Ok(d.parse().unwrap_or(0.0));
    }
    Err(format!("Invalid duration format: \"{d}\""))
}

/// An org-duration-format value.
#[derive(Clone, Debug, PartialEq)]
pub enum Format {
    HMm,
    HMmSs,
    Units(Vec<Item>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// (UNIT . REQUIRED?)
    Unit(String, bool),
    /// (special . h:mm) / (special . h:mm:ss)
    Mixed(bool),
    /// (special . PRECISION)
    Precision(usize),
    Compact,
}

impl Format {
    /// Read a Lisp value of org-duration-format.
    pub fn from_sexp(v: &Sexp) -> Result<Format, String> {
        match v.sym() {
            Some("h:mm") => return Ok(Format::HMm),
            Some("h:mm:ss") => return Ok(Format::HMmSs),
            _ => {}
        }
        let l = v
            .list()
            .ok_or_else(|| format!("Invalid duration format specification: {v}"))?;
        let mut items = vec![];
        for e in l {
            if e.sym() == Some("compact") {
                items.push(Item::Compact);
                continue;
            }
            let car = e
                .car()
                .ok_or_else(|| format!("Invalid duration format specification: {v}"))?;
            let cdr = e.cdr();
            if car.sym() == Some("special") {
                let val = match &cdr {
                    Sexp::List(l) if l.len() == 1 => l[0].clone(),
                    x => x.clone(),
                };
                items.push(match (&val, val.sym()) {
                    (_, Some("h:mm")) => Item::Mixed(false),
                    (_, Some("h:mm:ss")) => Item::Mixed(true),
                    (Sexp::Int(n), _) if *n >= 0 => Item::Precision(*n as usize),
                    _ => return Err(format!("Unknown formatting directive: {val}")),
                });
            } else if let Some(u) = car.str() {
                items.push(Item::Unit(u.to_owned(), cdr.truthy()));
            }
        }
        Ok(Format::Units(items))
    }

    /// org-duration-format's value.
    pub fn option() -> Format {
        sexp::option("org-duration-format")
            .and_then(|v| Format::from_sexp(&v).ok())
            .unwrap_or(Format::Units(vec![
                Item::Unit("d".into(), false),
                Item::Mixed(false),
            ]))
    }
}

/// Emacs `/` on a modifier: integer division when both are integers.
fn div_floor(m: f64, modi: f64) -> f64 {
    if modi.fract() == 0.0 {
        (m.floor() / modi).floor()
    } else {
        m.floor() / modi
    }
}

/// org-duration-from-minutes (FMT None: org-duration-format).
pub fn from_minutes(minutes: f64, fmt: Option<&Format>, canonical: bool) -> Result<String, String> {
    if minutes < 0.0 {
        return Ok(format!("-{}", from_minutes(minutes.abs(), fmt, canonical)?));
    }
    let owned;
    let fmt = match fmt {
        Some(f) => f,
        None => {
            owned = Format::option();
            &owned
        }
    };
    match fmt {
        Format::HMm => Ok(format!(
            "{}:{:02}",
            (minutes / 60.0).trunc() as i64,
            minutes.rem_euclid(60.0).trunc() as i64
        )),
        Format::HMmSs => {
            let whole = minutes.floor();
            let secs = (60.0 * minutes).rem_euclid(60.0);
            Ok(format!(
                "{}:{:02}",
                from_minutes(whole, Some(&Format::HMm), false)?,
                secs.trunc() as i64
            ))
        }
        Format::Units(items) => {
            if let Some(hms) = items.iter().find_map(|i| {
                if let Item::Mixed(s) = i {
                    Some(*s)
                } else {
                    None
                }
            }) {
                let mode = if hms { Format::HMmSs } else { Format::HMm };
                let mut truncated = vec![];
                for i in items {
                    if let Item::Unit(u, _) = i
                        && modifier(u, canonical)? > 60.0
                    {
                        truncated.push(i.clone());
                    }
                }
                let mut min_mod: Option<f64> = None;
                for i in &truncated {
                    if let Item::Unit(u, _) = i {
                        let m = modifier(u, canonical)?;
                        min_mod = Some(min_mod.map_or(m, |x: f64| x.min(m)));
                    }
                }
                return match min_mod {
                    Some(mm) if minutes >= mm => {
                        let units_part = mm * div_floor(minutes, mm);
                        let rest = minutes - units_part;
                        let compact = items.contains(&Item::Compact);
                        Ok(format!(
                            "{}{}{}",
                            from_minutes(units_part, Some(&Format::Units(truncated)), canonical)?,
                            if compact { "" } else { " " },
                            from_minutes(rest, Some(&mode), false)?
                        ))
                    }
                    _ => from_minutes(minutes, Some(&mode), canonical),
                };
            }
            let precision = items.iter().find_map(|i| {
                if let Item::Precision(p) = i {
                    Some(*p)
                } else {
                    None
                }
            });
            let mut selected: Vec<(String, bool, f64)> = vec![];
            for i in items {
                if let Item::Unit(u, r) = i {
                    selected.push((u.clone(), *r, modifier(u, canonical)?));
                }
            }
            selected.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
            let sep = if items.contains(&Item::Compact) {
                ""
            } else {
                " "
            };
            let Some(last) = selected.last().cloned() else {
                return Err("Invalid duration format specification".into());
            };
            if let Some(p) = precision {
                let (u, _, m) = selected
                    .iter()
                    .find(|(_, req, m)| *req || *m <= minutes)
                    .cloned()
                    .unwrap_or(last);
                return Ok(format!("{:.*}{u}", p, minutes / m));
            }
            let mut left = minutes;
            let mut out = String::new();
            for (u, req, m) in &selected {
                if *m <= left {
                    let v = (left / m).floor();
                    left -= v * m;
                    out.push_str(&format!("{sep}{}{u}", v as i64));
                } else if *req {
                    out.push_str(&format!("{sep}0{u}"));
                }
            }
            let out = out.trim();
            Ok(if out.is_empty() {
                format!("0{}", last.0)
            } else {
                out.to_owned()
            })
        }
    }
}

/// org-duration-h:mm-only-p: Some(HMm/HMmSs) when every time is H:MM
/// (or H:MM:SS), None when one uses units.
pub fn hmm_only(times: &[&str]) -> Option<Format> {
    let r = regexps();
    let mut hms = false;
    for t in times {
        if r.full.is_match(t) || r.mixed.is_match(t) {
            return None;
        }
        if !hms && hmmss_re().is_match(t) {
            hms = true;
        }
    }
    Some(if hms { Format::HMmSs } else { Format::HMm })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_units(units: &str, f: impl FnOnce()) {
        crate::org::options::put("org-duration-units", toml::Value::String(units.into()));
        f();
        crate::org::options::set(toml::Table::new());
    }

    fn fmt(s: &str) -> Format {
        Format::from_sexp(&sexp::read(s).unwrap()).unwrap()
    }

    #[test]
    fn to_minutes_spec() {
        assert!(to_minutes("1:2", false).is_err());
        assert_eq!(to_minutes("1:01", false), Ok(61.0));
        assert_eq!(to_minutes("1:20:30", false), Ok(80.5));
        assert_eq!(to_minutes("2h 10min", false), Ok(130.0));
        assert_eq!(to_minutes("1d 1:02", false), Ok(1502.0));
        assert_eq!(to_minutes("2.5h", false), Ok(150.0));
        assert_eq!(to_minutes("2", false), Ok(2.0));
        assert_eq!(to_minutes("2.5", false), Ok(2.5));
        assert_eq!(to_minutes("", false), Ok(0.0));
        with_units("'((\"longmin\" . 2))", || {
            assert_eq!(to_minutes("2longmin", false), Ok(4.0));
            assert!(to_minutes("2longmin", true).is_err());
        });
        with_units("'((\"h\" . 61))", || {
            assert_eq!(to_minutes("1h", false), Ok(61.0));
            assert_eq!(to_minutes("1h", true), Ok(60.0));
        });
    }

    #[test]
    fn from_minutes_spec() {
        let f = |m: f64, s: &str| from_minutes(m, Some(&fmt(s)), false).unwrap();
        assert_eq!(f(60.0, "h:mm"), "1:00");
        assert_eq!(f(61.5, "h:mm:ss"), "1:01:30");
        assert_eq!(f(61.5, "h:mm"), "1:01");
        assert_eq!(f(60.0, "((\"h\" . nil) (\"min\" . nil))"), "1h");
        assert_eq!(f(60.0, "((\"h\" . nil) (\"min\" . t))"), "1h 0min");
        assert_eq!(f(50.0, "((\"h\" . nil) (\"min\" . nil))"), "50min");
        assert_eq!(f(50.0, "((\"h\" . t) (\"min\" . t))"), "0h 50min");
        assert_eq!(f(1450.0, "((\"d\" . nil) (special . h:mm))"), "1d 0:10");
        assert_eq!(
            f(1452.5, "((\"d\" . nil) (special . h:mm:ss))"),
            "1d 0:12:30"
        );
        assert_eq!(f(90.0, "((\"h\" . nil) (special . 1))"), "1.5h");
        assert_eq!(f(90.0, "((\"h\" . nil) (special . 2))"), "1.50h");
        assert_eq!(
            f(40.0, "((\"h\" . t) (\"min\" . nil) (special . 1))"),
            "0.7h"
        );
        assert_eq!(
            f(40.0, "((\"h\" . nil) (\"min\" . nil) (special . 1))"),
            "40.0min"
        );
        assert_eq!(
            f(0.5, "((\"h\" . nil) (\"min\" . nil) (special . 1))"),
            "0.5min"
        );
        assert_eq!(f(50.0, "((\"h\" . t) (\"min\" . t) compact)"), "0h50min");
        assert_eq!(
            f(1450.0, "((\"d\" . nil) (special . h:mm) compact)"),
            "1d0:10"
        );
        assert_eq!(
            from_minutes(-90.0, Some(&Format::HMm), false).unwrap(),
            "-1:30"
        );
        // The default format.
        assert_eq!(from_minutes(1450.0, None, false).unwrap(), "1d 0:10");
        assert_eq!(from_minutes(50.0, None, false).unwrap(), "0:50");
    }

    #[test]
    fn predicates() {
        for d in [
            "3:12",
            "123:12",
            "1:23:45",
            "3d 3h 4min",
            "3d3h4min",
            "3d 13:35",
            "3d13:35",
            "2.35h",
            "2 h",
        ] {
            assert!(is_duration(d), "{d}");
        }
        for d in ["1minute", "3::12", "3:2", "3:12:4", "3d 13:35 13h"] {
            assert!(!is_duration(d), "{d}");
        }
        with_units("'((\"minute\" . 1))", || assert!(is_duration("2minute")));
        assert_eq!(hmm_only(&["123:31", "1:00"]), Some(Format::HMm));
        assert_eq!(hmm_only(&["123:32", "1h"]), None);
        assert_eq!(hmm_only(&["3:33", "1:23:45"]), Some(Format::HMmSs));
    }
}
