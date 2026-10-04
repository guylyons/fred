//! magit-bisect.el: start, mark, skip, reset and run a bisect.
use super::branch::Next;
use super::repo::Repo;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Start,
    /// s: start (when needed) and run a script.
    Run,
    Bad,
    Good,
    /// m: ask which term to mark HEAD with.
    Mark,
    Skip,
    Reset,
}

fn rev(v: &str) -> Result<&str, String> {
    if v.is_empty() || v.starts_with('-') || v.chars().any(char::is_control) {
        return Err(format!("invalid revision {v:?}"));
    }
    Ok(v)
}

impl Repo {
    fn git_file(&self, name: &str) -> Option<std::path::PathBuf> {
        let p = self.read(&["rev-parse", "--git-path", name]).ok()?;
        Some(self.root.join(String::from_utf8_lossy(&p).trim()))
    }
    /// magit-bisect-in-progress-p.
    pub fn bisecting(&self) -> bool {
        self.git_file("BISECT_LOG").is_some_and(|p| p.exists())
    }
    /// magit-bisect-terms: (new/bad, old/good).
    pub fn bisect_terms(&self) -> Option<(String, String)> {
        let text = std::fs::read_to_string(self.git_file("BISECT_TERMS")?).ok()?;
        let mut lines = text.lines();
        Some((lines.next()?.to_owned(), lines.next()?.to_owned()))
    }
    pub fn bisect_prompts(&self, op: &Op) -> (Vec<String>, Vec<String>) {
        match op {
            Op::Start => (
                vec![
                    "Start bisect with bad revision (default HEAD): ".into(),
                    "Good revision: ".into(),
                ],
                vec!["HEAD".into(), String::new()],
            ),
            Op::Run if !self.bisecting() => (
                vec![
                    "Bisect shell command: ".into(),
                    "Start bisect with bad revision (default HEAD): ".into(),
                    "Good revision: ".into(),
                ],
                vec![String::new(), "HEAD".into(), String::new()],
            ),
            Op::Run => (vec!["Bisect shell command: ".into()], vec![String::new()]),
            Op::Mark => {
                let (new, old) = self.bisect_terms().unwrap_or(("bad".into(), "good".into()));
                (
                    vec![format!("Mark HEAD as {new} ([n]ew) or {old} ([o]ld): ")],
                    vec![String::new()],
                )
            }
            Op::Reset => (vec!["Reset bisect? (y or n) ".into()], vec![String::new()]),
            _ => (vec![], vec![]),
        }
    }
    /// magit-bisect-start--assert.
    fn bisect_assert(&self, bad: &str, good: &str) -> Result<(), String> {
        self.read(&["merge-base", "--end-of-options", bad, good])
            .map_err(|_| {
                format!("Good `{good}' or merge-base has to be an ancestor of bad `{bad}'")
            })?;
        if self.read(&["diff", "--quiet"]).is_err()
            || self.read(&["diff", "--cached", "--quiet"]).is_err()
        {
            return Err("Cannot bisect with uncommitted changes".into());
        }
        Ok(())
    }
    pub fn bisect_step(&self, op: Op, a: &[String], args: &[String]) -> Result<Next, String> {
        let at = |i: usize| a.get(i).map(String::as_str).unwrap_or("");
        let bisect = |words: Vec<String>| {
            let mut argv = vec!["bisect".to_owned()];
            argv.extend(words);
            Ok(Next::Git(argv))
        };
        let terms = || {
            self.bisect_terms()
                .ok_or_else(|| "Not bisecting".to_owned())
        };
        match op {
            Op::Start => {
                if self.bisecting() {
                    return Err("Already bisecting".into());
                }
                let (bad, good) = (rev(at(0))?, rev(at(1))?);
                self.bisect_assert(bad, good)?;
                let mut words = vec!["start".to_owned()];
                words.extend(args.iter().cloned());
                // Revisions precede "--"; anything after it is a pathspec.
                words.extend([bad.into(), good.into(), "--".into()]);
                bisect(words)
            }
            Op::Run => {
                let cmd = at(0).trim();
                if cmd.is_empty() {
                    return Err("A shell command is required".into());
                }
                if !self.bisecting() {
                    let (bad, good) = (rev(at(1))?, rev(at(2))?);
                    self.bisect_assert(bad, good)?;
                    let mut start = vec!["bisect", "start"];
                    start.extend(args.iter().map(String::as_str));
                    start.extend([bad, good, "--"]);
                    self.read(&start)?;
                }
                bisect(vec!["run".into(), "sh".into(), "-c".into(), cmd.into()])
            }
            Op::Bad => bisect(vec![terms()?.0]),
            Op::Good => bisect(vec![terms()?.1]),
            Op::Mark => {
                let (new, old) = terms()?;
                match at(0) {
                    "n" => bisect(vec![new]),
                    "o" => bisect(vec![old]),
                    _ => Err("Answer n or o".into()),
                }
            }
            Op::Skip => {
                terms()?;
                bisect(vec!["skip".into()])
            }
            Op::Reset => {
                if !matches!(at(0), "y" | "yes") {
                    return Err("Abort".into());
                }
                let result = bisect(vec!["reset".into()]);
                if let Some(p) = self.git_file("BISECT_CMD_OUTPUT") {
                    let _ = std::fs::remove_file(p);
                }
                result
            }
        }
    }
}
