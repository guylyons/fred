//! Agenda tests: views over temp files with a fixed date, compared with
//! the output of upstream Org (Emacs 30, `emacs --batch -Q`, the pinned
//! checkout) for the same files and options, and commands acting on the
//! source entries.

use crate::config::Config;
use crate::key::parse_keys;
use crate::org::options;
use crate::org::time::civil::{self, Tm};
use crate::session::Session;

const A_ORG: &str = "#+TODO: TODO NEXT | DONE
* TODO Write report :work:
  SCHEDULED: <2026-10-06 Tue>
* TODO Call Bob
  DEADLINE: <2026-10-09 Fri>
* Meeting :meet:
  <2026-10-07 Wed 10:00-11:00>
* DONE Finished thing
  CLOSED: [2026-10-05 Mon 12:00] SCHEDULED: <2026-10-05 Mon>
* NEXT Overdue
  SCHEDULED: <2026-10-01 Thu>
* TODO [#A] Old deadline
  DEADLINE: <2026-10-03 Sat>
* Range
  <2026-10-06 Tue>--<2026-10-08 Thu>
* Anniv
  <%%(diary-float 10 1 2)>
* TODO Unscheduled task :home:
** TODO Child
* TODO Lunch at 12:30
  SCHEDULED: <2026-10-06 Tue 12:30>
* Weekly review
  <2026-09-29 Tue +1w>
* Project :proj:
** TODO [#B] Sub task
   CLOCK: [2026-10-06 Tue 08:00]--[2026-10-06 Tue 09:00] =>  1:00
** DONE Sub done
* Archived :ARCHIVE:
** TODO Hidden task
   SCHEDULED: <2026-10-06 Tue>
";

pub struct T {
    pub dir: tempfile::TempDir,
    pub s: Session,
}

impl T {
    /// A session on `src.txt` with agenda files `files`, on 2026-10-06 09:30.
    pub fn new(files: &[(&str, &str)]) -> T {
        let dir = tempfile::tempdir().unwrap();
        for (n, c) in files {
            std::fs::write(dir.path().join(n), c).unwrap();
        }
        std::fs::write(dir.path().join("src.txt"), "hello").unwrap();
        let (s, _) = Session::open(
            Some(dir.path().join("src.txt")),
            &Config::default(),
            &dir.path().join("swap"),
        )
        .unwrap();
        let agenda: Vec<toml::Value> = files
            .iter()
            .filter(|(n, _)| n.ends_with(".org"))
            .map(|(n, _)| toml::Value::String(dir.path().join(n).display().to_string()))
            .collect();
        options::put("org-agenda-files", toml::Value::Array(agenda));
        options::put(
            "org-agenda-time-grid",
            toml::Value::String(
                "'((daily today require-timed) (800 1000 1200 1400 1600 1800 2000) \"......\" \"----------------\")"
                    .into(),
            ),
        );
        options::put(
            "org-agenda-show-current-time-in-grid",
            toml::Value::Boolean(false),
        );
        options::put(
            "org-agenda-block-separator",
            toml::Value::Integer('=' as i64),
        );
        options::put(
            "diary-file",
            toml::Value::String(dir.path().join("nodiary").display().to_string()),
        );
        let now = Tm::at(2026, 10, 6, 9, 30);
        civil::set_now(Some(now));
        crate::org::set_now(Some(now.epoch()));
        super::set_width(Some(79));
        super::set_restriction(None);
        T { dir, s }
    }

    pub fn keys(&mut self, k: &str) {
        for key in parse_keys(k) {
            self.s.handle_key(key);
        }
    }

    pub fn text(&self) -> String {
        self.s.ed.buf.text()
    }

    pub fn file(&self, n: &str) -> String {
        let p = self.dir.path().join(n);
        self.s.org_text(&p).unwrap()
    }

    /// Put the cursor on the first agenda line containing `s`.
    pub fn goto(&mut self, s: &str) {
        let l = (0..self.s.ed.line_count())
            .find(|&l| self.s.ed.buf.line(l).contains(s))
            .unwrap_or_else(|| panic!("no line with {s:?} in\n{}", self.text()));
        self.s.ed.set_cursor(l, 0);
    }

    pub fn msg(&self) -> String {
        self.s
            .ed
            .msg
            .as_ref()
            .map(|m| m.0.clone())
            .unwrap_or_default()
    }
}

/// `emacs --batch -Q` with run.el: (org-agenda nil "a"), same options.
const WEEK: &str = "Week-agenda (W41):
Monday      5 October 2026 W41
  a:          Scheduled:  DONE Finished thing
Tuesday     6 October 2026
               8:00...... ----------------
              10:00...... ----------------
              12:00...... ----------------
  a:          12:30...... Scheduled:  TODO Lunch at\x20
              14:00...... ----------------
              16:00...... ----------------
              18:00...... ----------------
              20:00...... ----------------
  a:           3 d. ago:  TODO [#A] Old deadline
  a:          Sched. 5x:  NEXT Overdue
  a:          Scheduled:  TODO Write report                              :work:
  a:          (1/3):  Range
  a:          Weekly review
  a:          In   3 d.:  TODO Call Bob
Wednesday   7 October 2026
  a:          10:00-11:00 Meeting                                        :meet:
  a:          (2/3):  Range
Thursday    8 October 2026
  a:          (3/3):  Range
Friday      9 October 2026
  a:          Deadline:   TODO Call Bob
Saturday   10 October 2026
Sunday     11 October 2026";

/// (org-agenda nil "t").
const TODO: &str = "Global list of TODO items of type: ALL
Press ‘N r’ (e.g. ‘0 r’) to search again: (0)[ALL] (1)DONE (2)NEXT (3)TODO
  a:          TODO [#A] Old deadline
  a:          TODO Write report                                          :work:
  a:          TODO Call Bob
  a:          NEXT Overdue
  a:          TODO Unscheduled task                                      :home:
  a:          TODO Child                                                :home::
  a:          TODO Lunch at 12:30
  a:          TODO [#B] Sub task                                        :proj::";

#[test]
fn week_agenda_matches_emacs() {
    let mut t = T::new(&[("a.org", A_ORG)]);
    t.keys(" oaa");
    assert_eq!(t.text(), WEEK, "{}", t.msg());
    // The cursor starts on today.
    assert_eq!(
        t.s.ed.buf.line(t.s.ed.cur.line),
        "Tuesday     6 October 2026"
    );
    assert!(t.s.ed.readonly);
}

#[test]
fn todo_list_matches_emacs() {
    let mut t = T::new(&[("a.org", A_ORG)]);
    t.keys(" oat");
    assert_eq!(t.text(), TODO, "{}", t.msg());
}

const H_ORG: &str = "* TODO Shave
  SCHEDULED: <2026-10-05 Mon .+2d/4d>
  :PROPERTIES:
  :STYLE:    habit
  :END:
  - State \"DONE\"       from \"TODO\"       [2026-10-03 Sat]
  - State \"DONE\"       from \"TODO\"       [2026-09-30 Wed]
  - State \"DONE\"       from \"TODO\"       [2026-09-27 Sun]
* TODO Exercise
  SCHEDULED: <2026-10-06 Tue .+1d>
  :PROPERTIES:
  :STYLE:    habit
  :END:
  - State \"DONE\"       from \"TODO\"       [2026-10-05 Mon]
  - State \"DONE\"       from \"TODO\"       [2026-10-04 Sun]
  - State \"DONE\"       from \"TODO\"       [2026-10-02 Fri]
* TODO Normal
  SCHEDULED: <2026-10-06 Tue>
";

#[test]
fn habit_graphs_match_emacs() {
    // test-org-habit: consistency graphs at org-habit-graph-column.
    let mut t = T::new(&[("h.org", H_ORG)]);
    options::put("org-modules", toml::Value::String("'(org-habit)".into()));
    options::put("org-agenda-span", toml::Value::String("day".into()));
    t.keys(" oaa");
    let want = [
        "Day-agenda (W41):".to_owned(),
        "Tuesday     6 October 2026".to_owned(),
        "  h:          Scheduled:  TODO Normal".to_owned(),
        format!(
            "{:<69}",
            "  h:          TODO Exercise                              * **!"
        ),
        format!(
            "{:<69}",
            "  h:          TODO Shave                            *  *  *  !"
        ),
    ]
    .join("\n");
    assert_eq!(t.text(), want);
    // Habits off: K toggles (org-habit-toggle-display-in-agenda), through `\`.
    t.keys("\\K");
    assert_eq!(t.text().lines().count(), 3, "{}", t.text());
    t.keys("\\K");
}

const CUSTOM: &str = r#"'(("x" "Tags work" tags "work|proj")
  ("y" "Search" search "report")
  ("w" "Search bool" search "+task -sub")
  ("z" "Todo NEXT" todo "NEXT")
  ("b" "Block" ((agenda "" ((org-agenda-span 'day))) (alltodo "")))
  ("p" "Proj tags-todo" tags-todo "proj"))"#;

fn custom(t: &mut T, keys: &str) -> String {
    options::put(
        "org-agenda-custom-commands",
        toml::Value::String(CUSTOM.into()),
    );
    t.keys(&format!(" oa{keys}"));
    t.text()
}

#[test]
fn tags_search_and_todo_views_match_emacs() {
    let mut t = T::new(&[("a.org", A_ORG)]);
    assert_eq!(
        custom(&mut t, "x"),
        "Headlines with TAGS match: work|proj
Press ‘C-u r’ to search again
  a:          TODO Write report                                          :work:
  a:          Project                                                    :proj:
  a:          TODO [#B] Sub task                                        :proj::
  a:          DONE Sub done                                             :proj::"
    );
    assert_eq!(
        custom(&mut t, "y"),
        "Search words: report
Press ‘[’, ‘]’ to add/sub word, ‘{’, ‘}’ to add/sub regexp, ‘C-u r’ for a fresh search
  a:          TODO Write report                                          :work:"
    );
    assert_eq!(
        custom(&mut t, "w"),
        "Search words: +task -sub
Press ‘[’, ‘]’ to add/sub word, ‘{’, ‘}’ to add/sub regexp, ‘C-u r’ for a fresh search
  a:          TODO Unscheduled task                                      :home:"
    );
    assert_eq!(
        custom(&mut t, "z"),
        "Global list of TODO items of type: NEXT
Press ‘N r’ (e.g. ‘0 r’) to search again: (0)[ALL] (1)DONE (2)NEXT (3)TODO
  a:          NEXT Overdue"
    );
    assert_eq!(
        custom(&mut t, "p"),
        "Headlines with TAGS match: proj
Press ‘C-u r’ to search again
  a:          TODO [#B] Sub task                                        :proj::"
    );
}

#[test]
fn block_agenda_matches_emacs() {
    let mut t = T::new(&[("a.org", A_ORG)]);
    let want = "Day-agenda (W41):
Tuesday     6 October 2026
               8:00...... ----------------
              10:00...... ----------------
              12:00...... ----------------
  a:          12:30...... Scheduled:  TODO Lunch at\x20
              14:00...... ----------------
              16:00...... ----------------
              18:00...... ----------------
              20:00...... ----------------
  a:           3 d. ago:  TODO [#A] Old deadline
  a:          Sched. 5x:  NEXT Overdue
  a:          Scheduled:  TODO Write report                              :work:
  a:          (1/3):  Range
  a:          Weekly review
  a:          In   3 d.:  TODO Call Bob

===============================================================================
Global list of TODO items of type: ALL
  a:          TODO [#A] Old deadline
  a:          TODO Write report                                          :work:
  a:          TODO Call Bob
  a:          NEXT Overdue
  a:          TODO Unscheduled task                                      :home:
  a:          TODO Child                                                :home::
  a:          TODO Lunch at 12:30
  a:          TODO [#B] Sub task                                        :proj::";
    assert_eq!(custom(&mut t, "b"), want);
}

#[test]
fn stuck_projects_match_emacs() {
    let mut t = T::new(&[("a.org", A_ORG)]);
    options::put(
        "org-stuck-projects",
        toml::Value::String(r#"'("+proj-ARCHIVE" ("NEXT") nil "")"#.into()),
    );
    t.keys(" oa#");
    assert_eq!(
        t.text(),
        "List of stuck projects:\x20
  a:          Project                                                    :proj:
  a:          TODO [#B] Sub task                                        :proj::
  a:          DONE Sub done                                             :proj::"
    );
}

#[test]
fn log_and_archives_modes_match_emacs() {
    let mut t = T::new(&[("a.org", A_ORG)]);
    options::put("org-agenda-span", toml::Value::String("day".into()));
    options::put("org-agenda-start-with-log-mode", toml::Value::Boolean(true));
    options::put(
        "org-agenda-start-with-archives-mode",
        toml::Value::Boolean(true),
    );
    options::put(
        "org-agenda-start-day",
        toml::Value::String("2026-10-05".into()),
    );
    t.keys(" oaa");
    assert_eq!(
        t.text(),
        "Day-agenda (W41):
Monday      5 October 2026 W41
               8:00...... ----------------
              10:00...... ----------------
  a:          12:00...... Closed:     DONE Finished thing
              12:00...... ----------------
              14:00...... ----------------
              16:00...... ----------------
              18:00...... ----------------
              20:00...... ----------------
  a:          Scheduled:  DONE Finished thing"
    );
    options::put(
        "org-agenda-start-day",
        toml::Value::String("2026-10-06".into()),
    );
    t.keys(" oaa");
    assert_eq!(
        t.text(),
        "Day-agenda (W41):
Tuesday     6 October 2026
  a:           8:00-9:00  Clocked:   (1:00) TODO [#B] Sub task          :proj::
               8:00...... ----------------
              10:00...... ----------------
              12:00...... ----------------
  a:          12:30...... Scheduled:  TODO Lunch at\x20
              14:00...... ----------------
              16:00...... ----------------
              18:00...... ----------------
              20:00...... ----------------
  a:           3 d. ago:  TODO [#A] Old deadline
  a:          Sched. 5x:  NEXT Overdue
  a:          Scheduled:  TODO Write report                              :work:
  a:          Scheduled:  TODO Hidden task                           :ARCHIVE::
  a:          (1/3):  Range
  a:          Weekly review
  a:          In   3 d.:  TODO Call Bob"
    );
}

// ---- commands on entries ----

#[test]
fn todo_from_agenda_changes_source_and_lines() {
    let mut t = T::new(&[("a.org", A_ORG)]);
    t.keys(" oaa");
    t.goto("TODO Write report");
    // org-todo cycles TODO -> NEXT (the file's #+TODO sequence).
    t.keys("t");
    assert!(
        t.file("a.org").contains("* NEXT Write report"),
        "{}\n{}",
        t.file("a.org"),
        t.msg()
    );
    // The agenda line follows (org-agenda-change-all-lines).
    let line = t.s.ed.buf.line(t.s.ed.cur.line);
    assert!(
        line.contains("Scheduled:  NEXT Write report"),
        "{line}\n{}",
        t.text()
    );
    assert!(line.ends_with(":work:"), "{line}");
    t.keys("t");
    assert!(t.file("a.org").contains("* DONE Write report"));
    assert!(
        t.text().contains("Scheduled:  DONE Write report"),
        "{}",
        t.text()
    );
    // org-agenda-undo (C-_) undoes the last remote change.
    t.keys("\\<C-_>");
    assert!(
        t.file("a.org").contains("* NEXT Write report"),
        "{}",
        t.msg()
    );
}

#[test]
fn todo_list_cycles_and_numeric_redo() {
    let mut t = T::new(&[("a.org", A_ORG)]);
    t.keys(" oat");
    t.goto("NEXT Overdue");
    t.keys("t");
    assert!(t.file("a.org").contains("* DONE Overdue"), "{}", t.msg());
    // `3 r` (through `\`): only TODO keyword 3 (TODO).
    t.keys("3\\r");
    assert!(
        t.text()
            .starts_with("Global list of TODO items of type: TODO"),
        "{}",
        t.text()
    );
    assert!(!t.text().contains("NEXT Overdue"));
}

#[test]
fn schedule_and_deadline_show_new_time() {
    let mut t = T::new(&[("a.org", A_ORG)]);
    t.keys(" oat");
    t.goto("TODO Unscheduled task");
    t.keys("s2026-10-08<Enter>");
    let f = t.file("a.org");
    assert!(
        f.contains("* TODO Unscheduled task :home:\nSCHEDULED: <2026-10-08 Thu>"),
        "{f}\n{}",
        t.msg()
    );
    let line = t.s.ed.buf.line(t.s.ed.cur.line);
    assert!(line.ends_with(" S => <2026-10-08 Thu> "), "{line:?}");
    t.goto("TODO Call Bob");
    t.keys("D2026-10-20<Enter>");
    assert!(
        t.file("a.org").contains("DEADLINE: <2026-10-20 Tue>"),
        "{}",
        t.msg()
    );
}

#[test]
fn date_shift_moves_timestamp() {
    let mut t = T::new(&[("a.org", A_ORG)]);
    t.keys(" oaa");
    t.goto("10:00-11:00 Meeting");
    // S-right through the Emacs-state escape: org-agenda-do-date-later.
    t.keys("\\<S-Right>");
    assert!(
        t.file("a.org").contains("<2026-10-08 Thu 10:00-11:00>"),
        "{}",
        t.msg()
    );
    assert!(
        t.msg()
            .contains("Time stamp changed to <2026-10-08 Thu 10:00-11:00>"),
        "{}",
        t.msg()
    );
}

#[test]
fn spans_navigation_and_today() {
    let mut t = T::new(&[("a.org", A_ORG)]);
    t.keys(" oaa");
    t.keys("L");
    assert!(
        t.text()
            .starts_with("Week-agenda (W42):\nMonday     12 October 2026 W42"),
        "{}",
        t.text()
    );
    assert!(t.text().contains("  a:          Anniv"), "{}", t.text());
    t.keys("H");
    assert!(t.text().starts_with("Week-agenda (W41):"));
    t.keys("vd");
    assert!(
        t.text()
            .starts_with("Day-agenda (W41):\nTuesday     6 October 2026"),
        "{}",
        t.text()
    );
    t.keys("L");
    assert!(
        t.text()
            .starts_with("Day-agenda (W41):\nWednesday   7 October 2026"),
        "{}",
        t.text()
    );
    t.keys(".");
    assert!(
        t.text().contains("Tuesday     6 October 2026"),
        "{}",
        t.text()
    );
    t.keys("vm");
    assert!(
        t.text()
            .starts_with("Month-agenda (W40-W44):\nThursday    1 October 2026"),
        "{}",
        t.text()
    );
    // Next/previous items skip date lines.
    t.keys("g");
    assert_eq!(
        t.s.ed.org_keys.len(),
        1,
        "{:?} {}",
        t.s.ed.vim.pending,
        t.msg()
    );
    t.keys("g");
    assert_eq!(t.s.ed.cur.line, 0, "{}", t.msg());
    t.keys("gj");
    let l = t.s.ed.cur.line;
    assert!(
        t.s.ed.buf.line(l).starts_with("  a:"),
        "{l}: {}",
        t.s.ed.buf.line(l)
    );
}

#[test]
fn filters_hide_lines() {
    let mut t = T::new(&[("a.org", A_ORG)]);
    t.keys(" oat");
    // org-agenda-filter (`\ /`): keep entries tagged work.
    t.keys("\\/+work<Enter>");
    let text = t.text();
    assert!(text.contains("TODO Write report"), "{text}");
    assert!(!text.contains("Call Bob"), "{text}");
    let v = t.s.ed.org_view.as_ref().unwrap();
    assert!(v.mode_name.contains("+work"), "{}", v.mode_name);
    t.keys("F");
    assert!(t.text().contains("Call Bob"));
    // Regexp filter (`\ =`).
    t.keys("\\=lunch<Enter>");
    assert!(
        t.text().contains("Lunch at 12:30") && !t.text().contains("Call Bob"),
        "{}",
        t.text()
    );
    t.keys("F");
    // Category filter (`\ <`) on a line, removed by pressing it again.
    t.goto("TODO Call Bob");
    t.keys("\\<lt>");
    assert!(t.text().contains("Call Bob"));
    t.keys("\\<lt>");
    assert!(t.text().contains("Child"));
}

#[test]
fn bulk_mark_and_todo_action() {
    let mut t = T::new(&[("a.org", A_ORG)]);
    t.keys(" oat");
    t.goto("TODO Call Bob");
    t.keys("m");
    t.goto("TODO Child");
    t.keys("m");
    assert!(
        t.text().contains("> a:          TODO Call Bob"),
        "{}",
        t.text()
    );
    assert!(t.msg().contains("2 entries marked"), "{}", t.msg());
    t.keys("BtDONE<Enter>");
    let f = t.file("a.org");
    assert!(
        f.contains("* DONE Call Bob") && f.contains("** DONE Child"),
        "{f}\n{}",
        t.msg()
    );
    assert!(t.msg().starts_with("Acted on 2 entries"), "{}", t.msg());
    assert!(!t.text().contains("> a:"));
}

#[test]
fn kill_asks_and_removes_entry() {
    let mut t = T::new(&[("a.org", A_ORG)]);
    t.keys(" oat");
    t.goto("TODO Call Bob");
    t.keys("dy");
    assert!(!t.file("a.org").contains("Call Bob"), "{}", t.msg());
    assert!(!t.text().contains("Call Bob"));
    assert_eq!(t.msg(), "Agenda item and source killed");
}

#[test]
fn visit_quit_and_exit() {
    let mut t = T::new(&[("a.org", A_ORG)]);
    t.keys(" oat");
    t.goto("TODO Child");
    t.keys("<Enter>");
    assert!(t.s.ed.path.as_ref().unwrap().ends_with("a.org"));
    assert_eq!(t.s.ed.buf.line(t.s.ed.cur.line), "** TODO Child");
    assert_eq!(t.s.ed.cur.byte, 8);
    // Back in the agenda, q returns to the buffer it was opened from.
    t.keys(" oat");
    t.keys("q");
    assert!(t.s.ed.path.as_ref().unwrap().ends_with("src.txt"));
    assert!(t.s.ed.org_view.is_none());
}

#[test]
fn restriction_lock_and_dispatcher_restriction() {
    let mut t = T::new(&[("a.org", A_ORG), ("b.org", "* TODO In b\n")]);
    // From a.org: restrict to the buffer with `<`.
    let a = t.dir.path().join("a.org");
    t.s.org_visit(&a, 0).unwrap();
    t.keys(" oa<lt>t");
    assert!(
        t.text().contains("Call Bob") && !t.text().contains("In b"),
        "{}",
        t.text()
    );
    t.keys("q");
    // Unrestricted again.
    t.keys(" oat");
    assert!(t.text().contains("In b"), "{}", t.text());
    t.keys("q");
    // A subtree lock (C-c C-x <) on "Project".
    t.s.org_visit(&a, 30).unwrap();
    t.goto("* Project");
    t.keys("<C-c><C-x><lt>");
    assert_eq!(t.msg(), "Locking agenda restriction to subtree");
    t.keys(" oat");
    assert!(
        t.text().contains("Sub task") && !t.text().contains("Call Bob"),
        "{}",
        t.text()
    );
    t.keys("q");
    t.keys("<C-c><C-x>>");
    t.keys(" oat");
    assert!(t.text().contains("Call Bob"), "{}", t.text());
}

#[test]
fn entry_text_and_follow_show() {
    let mut t = T::new(&[(
        "a.org",
        "* TODO Task\n  Some notes here.\n  :PROPERTIES:\n  :X: 1\n  :END:\n  More.\n",
    )]);
    t.keys(" oat");
    t.keys("\\E");
    assert_eq!(
        t.text().lines().skip(2).collect::<Vec<_>>(),
        vec![
            "  a:          TODO Task",
            "    > Some notes here.",
            "    > More."
        ],
        "{}",
        t.text()
    );
    t.keys("\\E");
    assert_eq!(t.text().lines().count(), 3);
}

#[test]
fn agenda_files_front_and_remove() {
    let mut t = T::new(&[("a.org", A_ORG)]);
    let b = t.dir.path().join("b.org");
    std::fs::write(&b, "* TODO In b\n").unwrap();
    t.s.org_visit(&b, 0).unwrap();
    t.keys("<C-c>[");
    assert!(
        t.msg().contains("added to front of list of agenda files"),
        "{}",
        t.msg()
    );
    let files = crate::org::agenda_files();
    assert!(files[0].ends_with("b.org"), "{files:?}");
    t.keys("<C-c>]");
    assert!(
        t.msg().starts_with("Removed from Org Agenda list"),
        "{}",
        t.msg()
    );
    assert!(
        !crate::org::agenda_files()
            .iter()
            .any(|f| f.ends_with("b.org"))
    );
}
