//! org-habit.el: habits (STYLE "habit" entries with `.+` style
//! repeaters), their urgency and the consistency graph.

use super::collect::Src;
use super::{Face, Habit, opt, opt_bool, opt_int};
use crate::org::props::Inherit;
use crate::org::time::{self, stamp};

/// org-habit is loaded: `org-habit` in org-modules.
pub fn loaded() -> bool {
    opt("org-modules").is_some_and(|v| {
        v.list()
            .unwrap_or(&[])
            .iter()
            .any(|x| x.str() == Some("org-habit"))
    })
}

/// org-is-habit-p.
pub fn is_habit(src: &Src, h: usize) -> bool {
    src.prop(h, "STYLE", Inherit::Selective).as_deref() == Some("habit")
}

/// The element's own :STYLE property is "habit".
pub fn is_habit_local(src: &Src, h: usize) -> bool {
    src.prop(h, "STYLE", Inherit::No).as_deref() == Some("habit")
}

/// org-habit-repeater-to-days.
fn to_days(value: i64, unit: &str) -> i64 {
    let f = match unit {
        "d" => 1.0,
        "w" => 7.0,
        "m" => 30.4,
        "y" => 365.25,
        _ => 0.0,
    };
    (value as f64 * f).floor() as i64
}

/// org-habit-parse-todo at heading `h` (None when it is not a valid habit).
pub fn parse(src: &Src, h: usize, _today: i64) -> Option<Habit> {
    parse_err(src, h).ok()
}

/// org-habit-parse-todo, with its errors.
pub fn parse_err(src: &Src, h: usize) -> Result<Habit, String> {
    let title = src.title(h);
    let sched = src
        .planning_get(h, time::Planning::Scheduled)
        .ok_or_else(|| format!("Habit {title} has no scheduled date"))?;
    let c = re!(r"(\.\+|\+\+|\+)([0-9]+)([dwmy])(?:/([0-9]+)([dwmy]))?").captures(&sched);
    let Some(c) = c else {
        return Err(format!(
            "Habit `{title}' has no scheduled repeat period or has an incorrect one"
        ));
    };
    let scheduled = stamp::time_string_to_absolute(&sched, None, "")?;
    let sr = to_days(c[2].parse().unwrap_or(0), &c[3]);
    if sr <= 0 {
        return Err(format!(
            "Habit {title} scheduled repeat period is less than 1d"
        ));
    }
    let mut deadline = None;
    let mut dr = None;
    if let (Some(v), Some(u)) = (c.get(4), c.get(5)) {
        let d = to_days(v.as_str().parse().unwrap_or(0), u.as_str());
        if d <= sr {
            return Err(format!(
                "Habit {title} deadline repeat period is less than or equal to scheduled repeat"
            ));
        }
        dr = Some(d);
        deadline = Some(scheduled + (d - sr));
    }
    // Done dates from the state change log (newest first in the entry).
    let maxdays =
        (opt_int("org-habit-preceding-days", 21) + opt_int("org-habit-following-days", 7)) as usize;
    let end = (h + 1..src.lines.len())
        .find(|&l| syntax_level(&src.lines[l]).is_some())
        .unwrap_or(src.lines.len());
    let done: Vec<String> = src
        .st
        .done_names()
        .iter()
        .map(|k| regex::escape(k))
        .collect();
    let re = regex::Regex::new(&format!(
        r#"^[ \t]*-[ \t]+(?:State "(?:{})".*\[([0-9]{{4}}-[0-9]{{2}}-[0-9]{{2}}[^\]\n]*)\]|CLOSING NOTE \[([0-9]{{4}}-[0-9]{{2}}-[0-9]{{2}}[^\]\n]*)\])"#,
        done.join("|")
    ))
    .map_err(|e| e.to_string())?;
    let reversed = opt_bool("org-log-states-order-reversed", true);
    let lines: Box<dyn Iterator<Item = usize>> = if reversed {
        Box::new(h + 1..end)
    } else {
        Box::new((h + 1..end).rev())
    };
    let mut dates = vec![];
    for l in lines {
        if dates.len() >= maxdays {
            break;
        }
        if let Some(c) = re.captures(&src.lines[l]) {
            let ts = c.get(1).or(c.get(2)).map_or("", |m| m.as_str());
            if let Ok(d) = stamp::time_string_to_absolute(ts, None, "") {
                dates.push(d);
            }
        }
    }
    dates.reverse();
    Ok(Habit {
        scheduled,
        repeat: sr,
        deadline,
        deadline_repeat: dr,
        done: dates,
        repeat_type: c[1].to_owned(),
    })
}

fn syntax_level(l: &str) -> Option<usize> {
    crate::org::syntax::level(l)
}

/// org-habit-deadline.
pub fn deadline(h: &Habit) -> i64 {
    h.deadline.unwrap_or(if h.deadline_repeat.is_some() {
        h.scheduled + h.repeat - 1
    } else {
        h.scheduled
    })
}

fn deadline_repeat(h: &Habit) -> i64 {
    h.deadline_repeat.unwrap_or(h.repeat)
}

/// org-habit-get-urgency.
pub fn urgency(h: &Habit, now: i64) -> i64 {
    let mut pri = 1000 + (now - h.scheduled) * 10;
    let d = deadline(h);
    if h.scheduled != d && now == d {
        pri += 50;
    }
    let slip = now - (d - 1);
    pri += if slip > 0 { slip * 100 } else { slip * 10 };
    pri
}

/// org-habit-get-faces: (past face, future face).
pub fn faces(h: &Habit, now: i64, scheduled_days: Option<i64>, donep: bool) -> (Face, Face) {
    let scheduled = scheduled_days.unwrap_or(h.scheduled);
    let deadline = match scheduled_days {
        Some(s) => s + (deadline_repeat(h) - h.repeat),
        None => deadline(h),
    };
    if now < scheduled {
        (Face::HabitClear, Face::HabitClearFuture)
    } else if now < deadline {
        (Face::HabitReady, Face::HabitReadyFuture)
    } else if now == deadline {
        if donep {
            (Face::HabitReady, Face::HabitReadyFuture)
        } else {
            (Face::HabitAlert, Face::HabitAlertFuture)
        }
    } else if opt_bool("org-habit-show-done-always-green", false) && donep {
        (Face::HabitReady, Face::HabitReadyFuture)
    } else {
        (Face::HabitOverdue, Face::HabitOverdueFuture)
    }
}

fn glyph(name: &str, def: char) -> char {
    match opt(name) {
        Some(crate::org::sexp::Sexp::Int(n)) => char::from_u32(n as u32).unwrap_or(def),
        Some(v) => v.str().and_then(|s| s.chars().next()).unwrap_or(def),
        None => def,
    }
}

/// org-habit-build-graph: the glyphs and their faces from `start` to
/// `end` (exclusive), `now` being today.
pub fn build_graph(h: &Habit, start: i64, now: i64, end: i64) -> Vec<(char, Face)> {
    let mut all = h.done.clone();
    all.sort_unstable();
    let mut done: &[i64] = &all;
    let scheduled = h.scheduled;
    let sr = h.repeat;
    let mut last_done = None;
    while let Some((&d, rest)) = done.split_first() {
        if d >= start {
            break;
        }
        last_done = Some(d);
        done = rest;
    }
    let completed = glyph("org-habit-completed-glyph", '*');
    let today_glyph = glyph("org-habit-today-glyph", '!');
    let mut out = vec![];
    let mut day = start;
    while day < end {
        let past = day < now;
        let todayp = day == now;
        let donep = done.first() == Some(&day);
        let faces = if past && last_done.is_none() && scheduled >= now {
            if all.first() == Some(&day) {
                (Face::HabitReady, Face::HabitReadyFuture)
            } else {
                (Face::HabitClear, Face::HabitClearFuture)
            }
        } else {
            let sched_then = match (past, last_done) {
                (true, Some(last)) => Some(if done.is_empty() {
                    scheduled
                } else if h.repeat_type == ".+" {
                    last + sr
                } else if h.repeat_type == "+" {
                    scheduled - done.len() as i64 * sr
                } else {
                    let first = all[0];
                    let shift = (scheduled - first).rem_euclid(sr);
                    let mut s = first + if shift == 0 { sr } else { shift };
                    if first != last {
                        for &d in &all[1..] {
                            s += (1 + (d - s).max(0) / sr) * sr;
                            if d == last {
                                break;
                            }
                        }
                    }
                    s
                }),
                _ => None,
            };
            faces(h, day, sched_then, donep)
        };
        let mut ch = ' ';
        let mut marked = false;
        if donep {
            ch = completed;
            marked = true;
            while done.first() == Some(&day) {
                last_done = Some(day);
                done = &done[1..];
            }
        } else if todayp {
            ch = today_glyph;
        }
        let mut face = if past || todayp { faces.0 } else { faces.1 };
        if past && face != Face::HabitOverdue && !marked {
            face = faces.1;
        }
        out.push((ch, face));
        day += 1;
    }
    out
}

/// The graph of a habit item for today (org-habit-insert-consistency-graphs).
pub fn graph(h: &Habit, today: i64) -> Vec<(char, Face)> {
    let before = opt_int("org-habit-preceding-days", 21);
    let after = opt_int("org-habit-following-days", 7);
    // Upstream's graph vector has one more (blank) cell than the days drawn.
    let mut g = build_graph(h, today - before, today, today + after);
    g.push((' ', Face::Default));
    g
}

/// Put the graph at org-habit-graph-column of `line`: (new line, the
/// byte range of each glyph and its face).
pub fn insert_graph(
    line: &str,
    h: &Habit,
    today: i64,
) -> (String, Vec<(std::ops::Range<usize>, Face)>) {
    let col = opt_int("org-habit-graph-column", 40) as usize;
    let g = graph(h, today);
    let mut head: String = line.chars().take(col).collect();
    let n = head.chars().count();
    if n < col {
        head.push_str(&" ".repeat(col - n));
    }
    let rest: String = line.chars().skip(col + g.len() + 1).collect();
    let mut spans = vec![];
    let mut out = head;
    for (c, f) in g {
        let a = out.len();
        out.push(c);
        spans.push((a..out.len(), f));
    }
    out.push_str(&rest);
    (out, spans)
}

/// org-habit-toggle-habits / org-habit-toggle-display-in-agenda.
pub fn toggle(ed: &mut crate::editor::Editor, all_today: bool) -> Result<(), String> {
    super::views::check_type(ed, &[super::Kind::Agenda])?;
    if all_today {
        let v = !opt_bool("org-habit-show-all-today", false);
        crate::org::options::put("org-habit-show-all-today", toml::Value::Boolean(v));
        if opt_bool("org-habit-show-habits", true) {
            super::views::redo(ed, false);
        }
        return Ok(());
    }
    let v = !opt_bool("org-habit-show-habits", true);
    crate::org::options::put("org-habit-show-habits", toml::Value::Boolean(v));
    super::views::redo(ed, false);
    ed.set_msg(format!("Habits turned {}", if v { "on" } else { "off" }));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::org::time::civil::Tm;

    fn d(y: i64, m: i64, dd: i64) -> i64 {
        Tm::date(y, m, dd).absolute()
    }

    fn src(text: &str) -> Src {
        Src::new("/tmp/h.org".into(), text, None)
    }

    #[test]
    fn parse_habit() {
        // test-org-habit/org-habit-parse-todo.
        let s = src(
            "* TODO Shave\n  SCHEDULED: <2009-10-17 Sat .+2d/4d>\n  :PROPERTIES:\n  :STYLE:    habit\n  :END:\n  - State \"DONE\"       from \"TODO\"       [2009-10-15 Thu]\n  - State \"DONE\"       from \"TODO\"       [2009-10-12 Mon]\n",
        );
        assert!(is_habit(&s, 0));
        let h = parse_err(&s, 0).unwrap();
        assert_eq!(h.scheduled, d(2009, 10, 17));
        assert_eq!(h.repeat, 2);
        assert_eq!(h.deadline, Some(d(2009, 10, 19)));
        assert_eq!(h.deadline_repeat, Some(4));
        assert_eq!(h.done, vec![d(2009, 10, 12), d(2009, 10, 15)]);
        assert_eq!(h.repeat_type, ".+");
        assert_eq!(deadline(&h), d(2009, 10, 19));
    }

    #[test]
    fn urgency_and_faces() {
        let h = Habit {
            scheduled: 100,
            repeat: 2,
            deadline: Some(102),
            deadline_repeat: Some(4),
            done: vec![],
            repeat_type: ".+".into(),
        };
        assert_eq!(urgency(&h, 100), 1000 + 10 * (100 - 101));
        assert_eq!(urgency(&h, 102), 1000 + 20 + 50 + 100);
        assert_eq!(faces(&h, 99, None, false).0, Face::HabitClear);
        assert_eq!(faces(&h, 101, None, false).0, Face::HabitReady);
        assert_eq!(faces(&h, 102, None, false).0, Face::HabitAlert);
        assert_eq!(faces(&h, 103, None, false).0, Face::HabitOverdue);
    }

    #[test]
    fn graph_glyphs() {
        let h = Habit {
            scheduled: 110,
            repeat: 1,
            deadline: None,
            deadline_repeat: None,
            done: vec![105, 108],
            repeat_type: ".+".into(),
        };
        let g = build_graph(&h, 104, 110, 112);
        let s: String = g.iter().map(|x| x.0).collect();
        assert_eq!(s, " *  * ! ");
    }
}
