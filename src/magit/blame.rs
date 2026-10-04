//! magit-blame.el: addition/echo blame of a visited file, drawn in Fred's gutter.
use super::repo::{Repo, label};
use crate::editor::{Editor, Mode};
use crate::ex::ExEffect;
use crate::key::{Key, KeyCode};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub rev: String,
    /// First line in the blamed file, 0-based.
    pub line: usize,
    pub lines: usize,
    pub orig_line: usize,
    pub orig_file: PathBuf,
    pub prev: Option<(String, PathBuf)>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Info {
    pub summary: String,
    pub author: String,
    pub committer_time: i64,
    pub committer_tz: String,
}
/// magit-blame-styles' default entries; Fred has no virtual lines, so headings
/// become a left margin and lines a chunk-boundary rule.
pub const STYLES: [&str; 3] = ["headings", "highlight", "lines"];
pub const HEADING_WIDTH: usize = 48;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blame {
    pub repo: Repo,
    pub file: PathBuf,
    pub args: Vec<String>,
    pub chunks: Vec<Chunk>,
    pub info: HashMap<String, Info>,
    pub style: usize,
    pub echo: bool,
    /// Buffer version the line numbers belong to.
    pub version: u64,
    pub was_readonly: bool,
}

impl Repo {
    /// `git blame --incremental`, parsed like magit-blame--parse-chunk.
    pub fn blame(
        &self,
        file: &Path,
        args: &[String],
    ) -> Result<(Vec<Chunk>, HashMap<String, Info>), String> {
        let mut argv: Vec<std::ffi::OsString> = vec!["blame".into(), "--incremental".into()];
        argv.extend(args.iter().map(Into::into));
        argv.push("--".into());
        argv.push(file.into());
        let out = self.run(&argv, None)?;
        let text = String::from_utf8_lossy(&out);
        let mut lines = text.lines();
        let (mut chunks, mut info) = (vec![], HashMap::<String, Info>::new());
        while let Some(head) = lines.next() {
            let mut f = head.split(' ');
            let (Some(rev), Some(orig), Some(fin), Some(n)) =
                (f.next(), f.next(), f.next(), f.next())
            else {
                return Err(format!("Blaming failed due to unexpected output: {head}"));
            };
            let num = |s: &str| {
                s.parse::<usize>()
                    .map_err(|_| format!("bad blame line {head}"))
            };
            let mut chunk = Chunk {
                rev: rev.into(),
                orig_line: num(orig)?,
                line: num(fin)?.saturating_sub(1),
                lines: num(n)?,
                orig_file: PathBuf::new(),
                prev: None,
            };
            let entry = info.entry(rev.to_owned()).or_default();
            for l in lines.by_ref() {
                let (key, value) = l.split_once(' ').unwrap_or((l, ""));
                match key {
                    "filename" => {
                        chunk.orig_file = value.into();
                        break;
                    }
                    "previous" => {
                        if let Some((r, f)) = value.split_once(' ') {
                            chunk.prev = Some((r.into(), f.into()));
                        }
                    }
                    "summary" => entry.summary = value.into(),
                    "author" => entry.author = value.into(),
                    "committer-time" => entry.committer_time = value.parse().unwrap_or(0),
                    "committer-tz" => entry.committer_tz = value.into(),
                    _ => (),
                }
            }
            chunks.push(chunk);
        }
        chunks.sort_by_key(|c| c.line);
        Ok((chunks, info))
    }
}

/// "%F %H:%M" in the commit's own zone.
// ponytail: upstream formats in Emacs' local zone; Fred has no tz database.
fn time(secs: i64, tz: &str) -> String {
    let offset = tz
        .get(1..5)
        .and_then(|d| Some(d[..2].parse::<i64>().ok()? * 3600 + d[2..].parse::<i64>().ok()? * 60))
        .map_or(0, |o| if tz.starts_with('-') { -o } else { o });
    let t = secs + offset;
    let (days, rem) = (t.div_euclid(86400), t.rem_euclid(86400));
    // Howard Hinnant's civil_from_days.
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60
    )
}

impl Blame {
    pub fn style(&self) -> &'static str {
        STYLES[self.style % STYLES.len()]
    }
    pub fn chunk_at(&self, line: usize) -> Option<&Chunk> {
        self.chunks
            .iter()
            .find(|c| line >= c.line && line < c.line + c.lines)
    }
    /// magit-blame heading-format "%-20a %C %s".
    pub fn heading(&self, chunk: &Chunk) -> String {
        if chunk.rev.bytes().all(|b| b == b'0') {
            return "Not Yet Committed".into();
        }
        let info = self.info.get(&chunk.rev).cloned().unwrap_or_default();
        format!(
            "{:<20} {} {}",
            label(Path::new(&info.author)),
            time(info.committer_time, &info.committer_tz),
            label(Path::new(&info.summary))
        )
    }
    /// The gutter text for a buffer line, if this style draws one.
    pub fn margin(&self, line: usize) -> Option<(usize, String)> {
        if self.echo {
            return None;
        }
        let chunk = self.chunk_at(line)?;
        match self.style() {
            "headings" => Some((
                HEADING_WIDTH,
                if chunk.line == line {
                    self.heading(chunk)
                } else {
                    String::new()
                },
            )),
            "lines" => Some((1, if chunk.line == line { "┌" } else { "│" }.into())),
            _ => None,
        }
    }
    pub fn width(&self) -> usize {
        if self.echo {
            return 0;
        }
        match self.style() {
            "headings" => HEADING_WIDTH,
            "lines" => 1,
            _ => 0,
        }
    }
    /// show-message: the summary of the chunk at point.
    pub fn message(&self, line: usize) -> Option<String> {
        // magit-blame-echo shows only the message.
        (self.echo || self.style() == "lines")
            .then(|| self.chunk_at(line).map(|c| self.heading(c)))?
    }
}

/// magit-blame-read-only-mode-map, minus SPC/DEL (Fred's leader) and M-w.
pub fn key(ed: &mut Editor, k: Key) -> bool {
    let Some(blame) = &ed.blame else {
        return false;
    };
    // Overlays that keep Normal mode (word-jump, explain) own their keys.
    if ed.mode != Mode::Normal
        || !ed.vim.pending.is_empty()
        || ed.zap.is_some()
        || ed.explain.is_some()
        || k.ctrl
        || k.alt
    {
        return false;
    }
    // ponytail: edits invalidate line mapping; upstream overlays move with text.
    if blame.version != ed.buf.version {
        quit(ed);
        ed.set_msg("Blame removed: buffer changed");
        return false;
    }
    if blame.echo {
        return false;
    }
    let line = ed.cur.line;
    let here = blame.chunk_at(line).cloned();
    let target = |forward: bool, same: bool| {
        let mut chunks: Vec<_> = blame
            .chunks
            .iter()
            .filter(|c| !same || here.as_ref().is_some_and(|h| h.rev == c.rev))
            .collect();
        if !forward {
            chunks.reverse();
        }
        chunks
            .into_iter()
            .find(|c| {
                if forward {
                    c.line > line
                } else {
                    c.line + c.lines <= line
                }
            })
            .map(|c| c.line)
    };
    let jump = match k.code {
        KeyCode::Char('n') => Some(target(true, false)),
        KeyCode::Char('N') => Some(target(true, true)),
        KeyCode::Char('p') => Some(target(false, false)),
        KeyCode::Char('P') => Some(target(false, true)),
        _ => None,
    };
    if let Some(jump) = jump {
        match jump {
            Some(l) => ed.set_cursor(l, 0),
            None => ed.set_msg("No more chunks"),
        }
        return true;
    }
    let action = match k.code {
        KeyCode::Enter
            if here
                .as_ref()
                .is_some_and(|c| c.rev.bytes().all(|b| b == b'0')) =>
        {
            ed.set_err("Not Yet Committed");
            return true;
        }
        KeyCode::Enter => here.map(|c| {
            super::Action::Answered(
                blame.repo.clone(),
                super::Question::Diff(super::diff::Op::ShowCommit),
                vec![c.rev],
                super::menu_arguments(ed, 'd'),
            )
        }),
        KeyCode::Char('c') => {
            cycle(ed);
            return true;
        }
        KeyCode::Char('q') => {
            quit(ed);
            return true;
        }
        KeyCode::Char('B') => {
            super::open_menu(ed, 'B');
            return true;
        }
        _ => return false,
    };
    if let Some(action) = action {
        ed.pending_effect = Some(ExEffect::Magit(action));
    }
    true
}
pub fn cycle(ed: &mut Editor) {
    if ed.blame.as_ref().is_some_and(|b| b.echo) {
        ed.set_msg("Blame echo has a single style");
        return;
    }
    if let Some(b) = &mut ed.blame {
        b.style = (b.style + 1) % STYLES.len();
        let style = b.style();
        ed.set_msg(format!("Blame style: {style}"));
    }
}
pub fn quit(ed: &mut Editor) {
    if let Some(b) = ed.blame.take() {
        ed.readonly = b.was_readonly;
    }
}
