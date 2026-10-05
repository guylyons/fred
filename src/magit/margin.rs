//! magit-margin.el: the author and date beside commits in log, reflog,
//! stash, refs, cherry and status buffers, drawn right-aligned by the UI.
use super::Kind;

/// The margin's date style (magit-cycle-margin-style).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Style {
    Age,
    AgeAbbreviated,
    /// A format-time-string (strftime) format.
    Format(String),
}

/// magit--right-margin-config: (INIT STYLE WIDTH AUTHOR AUTHOR-WIDTH), plus
/// magit-log-margin-show-shortstat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Margin {
    pub shown: bool,
    pub style: Style,
    pub details: bool,
    pub details_width: usize,
    pub shortstat: bool,
}

/// What a commit row's margin shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub author: String,
    pub time: i64,
    /// magit-log-format-shortstat-margin's text, when that style is on.
    pub stat: Option<String>,
}

/// The default time format (magit-margin-default-time-format).
const TIME_FORMAT: &str = "%Y-%m-%d %H:%M ";

impl Margin {
    /// magit--right-margin-option's defaults: magit-log-margin and the
    /// options derived from it. None where upstream has no margin.
    pub fn for_kind(kind: &Kind) -> Option<Margin> {
        use super::options;
        let (option, shown, details) = match kind {
            Kind::Log(..) | Kind::FileLog(..) => ("magit-log-margin", true, true),
            Kind::Cherry(..) => ("magit-cherry-margin", true, true),
            Kind::Reflog(_) => ("magit-reflog-margin", true, false),
            Kind::Stashes => ("magit-stashes-margin", true, false),
            Kind::Refs(..) => ("magit-refs-margin", false, false),
            Kind::Status => ("magit-status-margin", false, false),
            _ => return None,
        };
        // The other margins default to magit-log-margin's style and widths.
        let log = Self::parse(options::value("magit-log-margin"));
        let mut m = Margin {
            shown: shown && log.as_ref().is_none_or(|l| l.shown),
            style: log.as_ref().map_or(Style::Age, |l| l.style.clone()),
            details,
            details_width: log.as_ref().map_or(18, |l| l.details_width),
            shortstat: false,
        };
        if option == "magit-log-margin"
            && let Some(l) = &log
        {
            m.shown = l.shown;
            m.details = l.details;
        }
        if let Some(own) = Self::parse(options::value(option)) {
            m = own;
        }
        Some(m)
    }
    /// A margin option's own value.
    pub fn parse_option(name: &str) -> Option<Margin> {
        Self::parse(super::options::value(name))
    }
    /// (INIT STYLE WIDTH AUTHOR AUTHOR-WIDTH) as a TOML array, e.g.
    /// `[true, "age", "magit-log-margin-width", true, 18]`.
    fn parse(v: Option<toml::Value>) -> Option<Margin> {
        let a = v?.as_array()?.clone();
        let style = match a.get(1).and_then(|s| s.as_str()) {
            Some("age") | None => Style::Age,
            Some("age-abbreviated") => Style::AgeAbbreviated,
            Some(f) => Style::Format(f.to_owned()),
        };
        Some(Margin {
            shown: a.first().and_then(|b| b.as_bool()).unwrap_or(true),
            style,
            details: a.get(3).and_then(|b| b.as_bool()).unwrap_or(false),
            details_width: a
                .get(4)
                .and_then(|n| n.as_integer())
                .map_or(18, |n| n.max(1) as usize),
            shortstat: false,
        })
    }
    /// magit-log-margin-width.
    pub fn width(&self) -> usize {
        if self.shortstat {
            return 16;
        }
        let details = if self.details {
            self.details_width + 1
        } else {
            0
        };
        details
            + match &self.style {
                Style::Format(f) => strftime(f, 0).chars().count(),
                // Two digits, a space, then the unit.
                Style::AgeAbbreviated => 2 + 1 + 1,
                Style::Age => 2 + 1 + 1 + longest_unit(),
            }
    }
    /// magit-cycle-margin-style: age, abbreviated age, then a date.
    pub fn cycle_style(&mut self) {
        self.style = match self.style {
            Style::Age => Style::AgeAbbreviated,
            Style::AgeAbbreviated => Style::Format(TIME_FORMAT.into()),
            Style::Format(_) => Style::Age,
        };
    }
    /// magit-log-format-author-margin (or the shortstat margin).
    pub fn text(&self, stamp: &Stamp, now: i64) -> String {
        if self.shortstat {
            return stamp.stat.clone().unwrap_or_default();
        }
        let mut out = String::new();
        if self.details {
            let ellipsis =
                super::options::string("magit-ellipsis", None).unwrap_or_else(|| "…".into());
            let name = truncate_author(&stamp.author, &ellipsis, self.details_width);
            out.push_str(&format!("{name:<w$} ", w = self.details_width));
        }
        let rest = self.width() - out.chars().count();
        match &self.style {
            Style::Format(f) => out.push_str(&strftime(f, stamp.time)),
            Style::AgeAbbreviated => {
                let (n, unit) = age(now - stamp.time, true);
                out.push_str(&format!("{n:>2}{unit:<w$}", w = rest.saturating_sub(2)));
            }
            Style::Age => {
                let (n, unit) = age(now - stamp.time, false);
                out.push_str(&format!("{n:>2} {unit:<w$}", w = rest.saturating_sub(3)));
            }
        }
        out
    }
}

/// magit--age-spec: (abbreviation, unit, units, seconds).
const AGE: [(char, &str, &str, i64); 7] = [
    ('Y', "year", "years", 31_556_952),
    ('M', "month", "months", 2_629_746),
    ('w', "week", "weeks", 604_800),
    ('d', "day", "days", 86_400),
    ('h', "hour", "hours", 3_600),
    ('m', "minute", "minutes", 60),
    ('s', "second", "seconds", 1),
];

fn longest_unit() -> usize {
    AGE.iter()
        .map(|(_, a, b, _)| a.len().max(b.len()))
        .max()
        .unwrap_or(0)
}

/// magit--age: the count of the largest unit that fits, rounded.
pub fn age(seconds: i64, abbreviate: bool) -> (i64, String) {
    let secs = seconds.abs() as f64;
    for (i, (c, one, many, weight)) in AGE.iter().enumerate() {
        if secs / *weight as f64 >= 1.0 || i == AGE.len() - 1 {
            let n = (secs / *weight as f64).round() as i64;
            let unit = if abbreviate {
                c.to_string()
            } else if n == 1 {
                (*one).to_owned()
            } else {
                (*many).to_owned()
            };
            return (n, unit);
        }
    }
    unreachable!()
}

/// format-time-string in local time.
pub fn strftime(format: &str, time: i64) -> String {
    let Ok(fmt) = std::ffi::CString::new(format) else {
        return String::new();
    };
    let t = time as libc::time_t;
    // SAFETY: `tm` is plain data that localtime_r fills in.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are valid for the call.
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return String::new();
    }
    let mut buf = [0u8; 128];
    // SAFETY: buf is writable for its length; fmt and tm are valid.
    let n = unsafe { libc::strftime(buf.as_mut_ptr().cast(), buf.len(), fmt.as_ptr(), &tm) };
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

/// magit-log-format-shortstat-margin: "N+ N-  N" from git's --shortstat.
pub fn shortstat(line: &str) -> String {
    let num = |what: &str| {
        line.split(", ")
            .find(|p| p.contains(what))
            .and_then(|p| p.split_whitespace().next())
            .map(str::to_owned)
    };
    let files = num("changed").unwrap_or_default();
    let add = num("insertion")
        .map(|n| format!("{n}+"))
        .unwrap_or_default();
    let del = num("deletion").map(|n| format!("{n}-")).unwrap_or_default();
    format!("{add:>5} {del:>5}{files:>4}")
}

/// The current time in seconds.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

impl super::repo::Repo {
    /// Author and date (and shortstat) of each commit, in one Git call each.
    pub fn stamps(
        &self,
        ids: &[String],
        committer: bool,
        stat: bool,
    ) -> Result<Vec<(String, Stamp)>, String> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        // magit-log-margin-show-committer-date: the author with the
        // committer's date.
        let format = if committer {
            "--format=%x1e%H%x1f%cN%x1f%ct"
        } else if super::options::flag("magit-log-margin-show-committer-date", false) {
            "--format=%x1e%H%x1f%aN%x1f%ct"
        } else {
            "--format=%x1e%H%x1f%aN%x1f%at"
        };
        let mut argv = vec!["log", "--no-walk=unsorted", "--no-color", format];
        if stat {
            argv.push("--shortstat");
        }
        argv.extend(ids.iter().map(String::as_str));
        argv.push("--");
        let out = self.read(&argv)?;
        let mut stamps = vec![];
        for record in String::from_utf8_lossy(&out).split('\x1e').skip(1) {
            let (head, rest) = record.split_once('\n').unwrap_or((record, ""));
            let f: Vec<&str> = head.split('\x1f').collect();
            let [id, author, time] = f[..] else {
                continue;
            };
            let stat = stat.then(|| {
                rest.lines()
                    .find(|l| l.contains("changed"))
                    .map(|l| shortstat(l.trim()))
                    .unwrap_or_default()
            });
            stamps.push((
                id.to_owned(),
                Stamp {
                    author: author.to_owned(),
                    time: time.trim().parse().unwrap_or(0),
                    stat,
                },
            ));
        }
        Ok(stamps)
    }
}

fn truncate_author(author: &str, ellipsis: &str, width: usize) -> String {
    if author.chars().count() <= width {
        return author.to_owned();
    }
    let keep = width.saturating_sub(ellipsis.chars().count());
    author
        .chars()
        .take(keep)
        .chain(ellipsis.chars())
        .take(width)
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn author_truncation_respects_width_even_with_long_ellipsis() {
        assert_eq!(
            super::truncate_author("abcdef", "very-long-ellipsis", 2),
            "ve"
        );
        assert_eq!(super::truncate_author("abcdef", "…", 3), "ab…");
        assert_eq!(super::truncate_author("ab", "…", 3), "ab");
    }
}
