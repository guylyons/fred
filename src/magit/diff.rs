//! magit-diff.el prefix suffixes and their generated diff buffers.
use super::repo::Repo;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Dwim,
    Range,
    Paths,
    Unstaged,
    Staged,
    Worktree,
    ShowCommit,
    ShowStash,
}

/// magit-diff-refresh actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refresh {
    Buffer,
    SwitchRange,
    Flip,
    /// magit-diff-toggle-file-filter.
    FileFilter,
}

impl Target {
    /// magit-diff-switch-range-type (A..B <-> A...B) and -flip-revs (B..A).
    pub fn refreshed(&self, how: Refresh) -> Result<Target, String> {
        let Target::Range(r) = self else {
            return Err("No range to change in this buffer".into());
        };
        let (a, dots, b) = match r.split_once("...") {
            Some((a, b)) => (a, "...", b),
            None => match r.split_once("..") {
                Some((a, b)) => (a, "..", b),
                None => return Err(format!("{r} is not a range")),
            },
        };
        Ok(Target::Range(match how {
            Refresh::SwitchRange => {
                format!("{a}{}{b}", if dots == ".." { "..." } else { ".." })
            }
            Refresh::Flip => format!("{b}{dots}{a}"),
            Refresh::Buffer | Refresh::FileFilter => r.clone(),
        }))
    }
}

/// What a generated diff buffer shows; kept so gr can recompute it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Unstaged,
    Staged,
    /// A range, or a single revision compared with the working tree.
    Range(String),
    Paths(PathBuf, PathBuf),
    Commit(String),
}
impl Target {
    pub fn title(&self) -> String {
        match self {
            Self::Unstaged => "Changes between index and working tree".into(),
            Self::Staged => "Changes between HEAD and index".into(),
            Self::Range(range) => format!("Changes in {}", super::repo::label(range.as_ref())),
            Self::Paths(a, b) => format!(
                "Changes between {} and {}",
                super::repo::label(a),
                super::repo::label(b)
            ),
            Self::Commit(id) => format!("Commit {}", &id[..id.len().min(12)]),
        }
    }
}

fn revision(value: &str) -> Result<&str, String> {
    if value.is_empty() || value.starts_with('-') || value.chars().any(char::is_control) {
        return Err(format!("invalid revision {value:?}"));
    }
    Ok(value)
}

impl Repo {
    /// Prompts a suffix needs when nothing at point supplies its value.
    pub fn diff_prompts(op: Op) -> Vec<String> {
        match op {
            Op::Dwim | Op::Range => vec!["Diff for range: ".into()],
            Op::Paths => vec!["First file: ".into(), "Second file: ".into()],
            Op::ShowCommit => vec!["Show commit: ".into()],
            Op::ShowStash => vec!["Show stash: ".into()],
            Op::Unstaged | Op::Staged | Op::Worktree => vec![],
        }
    }
    /// Turn prompt answers into a target, validating revisions and files.
    pub fn diff_target(&self, op: Op, answers: &[String]) -> Result<Target, String> {
        let answer = |i: usize| answers.get(i).map(String::as_str).ok_or("missing answer");
        Ok(match op {
            // Passed through like magit-diff-range (HEAD^!, A^@ ...); Git reports bad ones.
            Op::Dwim | Op::Range => Target::Range(revision(answer(0)?)?.into()),
            Op::ShowCommit => {
                let rev = format!("{}^{{commit}}", revision(answer(0)?)?);
                let id = self
                    .read(&["rev-parse", "--verify", "-q", "--end-of-options", &rev])
                    .map_err(|_| format!("unknown commit {:?}", answer(0).unwrap_or("")))?;
                Target::Commit(String::from_utf8_lossy(&id).trim().into())
            }
            Op::Paths => {
                let file = |i: usize| -> Result<PathBuf, String> {
                    let path = self.root.join(answer(i)?);
                    if !path.is_file() {
                        return Err(format!("no such file {:?}", answer(i)?));
                    }
                    Ok(path)
                };
                Target::Paths(file(0)?, file(1)?)
            }
            Op::Unstaged | Op::Staged | Op::Worktree | Op::ShowStash => {
                return Err("this diff does not read a value".into());
            }
        })
    }
    pub fn diff_output(&self, target: &Target, args: &[String]) -> Result<Vec<u8>, String> {
        // Unquoted names, so visiting can read non-ASCII paths.
        let mut argv: Vec<std::ffi::OsString> = vec!["-c".into(), "core.quotePath=false".into()];
        let show = matches!(target, Target::Commit(_));
        argv.push(if show { "show" } else { "diff" }.into());
        // magit-insert-diff/revision always ask for the patch, even with --stat.
        argv.push("-p".into());
        argv.push("--no-color".into());
        // Fixed prefixes so visiting can read file names (diff.noprefix etc.).
        argv.push("--src-prefix=a/".into());
        argv.push("--dst-prefix=b/".into());
        if *target == Target::Staged {
            argv.push("--cached".into());
        }
        if matches!(target, Target::Paths(..)) {
            argv.push("--no-index".into());
        }
        // "-- FILE" entries (magit:--) limit the diff to files.
        let files: Vec<&str> = args.iter().filter_map(|a| a.strip_prefix("-- ")).collect();
        // --show-signature only applies to commits.
        argv.extend(
            args.iter()
                .filter(|a| !a.starts_with("-- "))
                .filter(|a| show || *a != "--show-signature")
                // magit-diff-paths passes no transient arguments.
                .filter(|_| !matches!(target, Target::Paths(..)))
                .map(Into::into),
        );
        match target {
            Target::Range(range) => argv.push(revision(range)?.into()),
            Target::Commit(id) => argv.push(revision(id)?.into()),
            _ => (),
        }
        argv.push("--".into());
        for f in files {
            argv.push(format!(":(literal){f}").into());
        }
        if let Target::Paths(a, b) = target {
            argv.push(a.into());
            argv.push(b.into());
        }
        let out = self
            .command()
            .args(&argv)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("git: {e}"))?;
        // diff --no-index exits 1 when the files differ.
        if out.status.success()
            || (matches!(target, Target::Paths(..)) && out.status.code() == Some(1))
        {
            Ok(out.stdout)
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
        }
    }
}

/// Where a diff line points (magit-diff-visit-file): the file on each side,
/// the 1-based line on each side, and whether the line was removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub file: PathBuf,
    pub old_file: PathBuf,
    pub line: usize,
    pub old_line: usize,
    pub removed: bool,
}

/// A file header path: drop the a/ or b/ prefix and the tab Git appends to
/// names with spaces (shown escaped as \t by Fred).
fn header_path(l: &str, prefix: &str) -> Option<String> {
    let p = l.strip_prefix(prefix)?;
    let p = p
        .strip_suffix("\\t")
        .or_else(|| p.strip_suffix('\t'))
        .unwrap_or(p);
    Some(p.to_owned())
}

/// Read the location of LINE in a rendered patch (a/ and b/ prefixes).
pub fn location(lines: &[&str], line: usize) -> Option<Location> {
    lines.get(line)?;
    // The hunk containing LINE (its header may be LINE itself), then the
    // file header above that hunk.
    let hunk = (0..=line).rev().find(|&i| lines[i].starts_with("@@"));
    let file_start = (0..=line).rev().find(|&i| lines[i].starts_with("diff "))?;
    // A hunk above this file's header belongs to the previous file.
    let hunk = hunk.filter(|&h| h > file_start);
    let mut new = None;
    let mut old = None;
    // On a file header, its names come before the first hunk.
    let end = hunk.unwrap_or_else(|| {
        (line + 1..lines.len())
            .find(|&i| lines[i].starts_with("@@") || lines[i].starts_with("diff "))
            .unwrap_or(lines.len())
    });
    for l in &lines[file_start..end] {
        if let Some(p) = header_path(l, "+++ b/") {
            new = Some(p);
        } else if let Some(p) = header_path(l, "--- a/") {
            old = Some(p);
        }
    }
    let new = new.or_else(|| old.clone())?;
    let old = old.unwrap_or_else(|| new.clone());
    let (file, old_file) = (PathBuf::from(new), PathBuf::from(old));
    let Some(h) = hunk else {
        return Some(Location {
            file,
            old_file,
            line: 1,
            old_line: 1,
            removed: false,
        });
    };
    // @@ -a,b +c,d @@, or combined @@@ -a -b +c @@@ with one column per parent.
    let header = lines[h];
    let ats = header.chars().take_while(|c| *c == '@').count();
    let columns = ats.saturating_sub(1).max(1);
    let num = |sign: char, nth: usize| {
        header
            .split_whitespace()
            .filter_map(|w| w.strip_prefix(sign))
            .nth(nth)
            .and_then(|w| w.split(',').next())
            .and_then(|n| n.parse::<usize>().ok())
    };
    let (mut new_n, mut old_n) = (num('+', 0)?, num('-', 0)?);
    // Upstream visits the first changed line from a hunk header.
    let target = if line == h {
        (h + 1..lines.len())
            .take_while(|&i| !lines[i].starts_with("@@") && !lines[i].starts_with("diff "))
            .find(|&i| lines[i].chars().take(columns).any(|c| c != ' '))
            .unwrap_or(h + 1)
            .min(lines.len().saturating_sub(1))
    } else {
        line
    };
    let marks = |l: &str| l.chars().take(columns).collect::<String>();
    for l in &lines[h + 1..target] {
        if l.starts_with('\\') {
            continue;
        }
        let m = marks(l);
        if !m.contains('-') {
            new_n += 1;
        }
        if !m.contains('+') {
            old_n += 1;
        }
    }
    let removed = marks(lines.get(target).copied().unwrap_or("")).contains('-');
    Some(Location {
        file,
        old_file,
        line: new_n.max(1),
        old_line: old_n.max(1),
        removed,
    })
}

/// The patch for the hunk at LINE (or the whole file on a file header) in
/// raw diff output lines. A single hunk of a renamed file is applied to the
/// new name without the rename (magit-diff-file-header's no-rename form).
pub fn hunk_patch(lines: &[&[u8]], line: usize) -> Result<Vec<u8>, String> {
    let starts = |i: usize, p: &[u8]| lines[i].starts_with(p);
    lines.get(line).ok_or("No hunk or file at point")?;
    let file_start = (0..=line)
        .rev()
        .find(|&i| starts(i, b"diff "))
        .ok_or("No hunk or file at point")?;
    if starts(file_start, b"diff --cc") || starts(file_start, b"diff --combined") {
        return Err("Cannot apply resolution hunks".into());
    }
    let file_end = (file_start + 1..lines.len())
        .find(|&i| starts(i, b"diff "))
        .unwrap_or(lines.len());
    let first_hunk = (file_start..file_end)
        .find(|&i| starts(i, b"@@"))
        .ok_or("No hunk to apply (binary or mode-only change)")?;
    let whole = line < first_hunk;
    let (from, to) = if whole {
        (first_hunk, file_end)
    } else {
        let h = (first_hunk..=line)
            .rev()
            .find(|&i| starts(i, b"@@"))
            .unwrap_or(first_hunk);
        let end = (h + 1..file_end)
            .find(|&i| starts(i, b"@@"))
            .unwrap_or(file_end);
        (h, end)
    };
    let header = &lines[file_start..first_hunk];
    let new_name: Option<&[u8]> = header.iter().find_map(|l| l.strip_prefix(b"+++ b/"));
    let mut patch = vec![];
    for l in header {
        let rename = [
            &b"similarity index"[..],
            b"dissimilarity index",
            b"rename from",
            b"rename to",
            b"copy from",
            b"copy to",
        ]
        .iter()
        .any(|p| l.starts_with(p));
        if !whole && rename {
            continue;
        }
        match (whole, new_name) {
            (false, Some(n)) if l.starts_with(b"diff --git ") => {
                patch.extend_from_slice(b"diff --git a/");
                patch.extend_from_slice(n);
                patch.extend_from_slice(b" b/");
                patch.extend_from_slice(n);
            }
            (false, Some(n)) if l.starts_with(b"--- a/") => {
                patch.extend_from_slice(b"--- a/");
                patch.extend_from_slice(n);
            }
            _ => patch.extend_from_slice(l),
        }
        patch.push(b'\n');
    }
    for l in &lines[from..to] {
        patch.extend_from_slice(l);
        patch.push(b'\n');
    }
    Ok(patch)
}

/// A --stat line: " path | 3 ++-" (path first, "|" then a count or "Bin").
fn stat_path(l: &str) -> Option<&str> {
    let (path, rest) = l.split_once(" | ")?;
    let rest = rest.trim_start();
    (rest.starts_with(|c: char| c.is_ascii_digit()) || rest.starts_with("Bin")).then(|| path.trim())
}

/// magit-jump-to-diffstat-or-diff: from a stat line to its file's diff,
/// otherwise to the stat line of the file at point (or the first one).
pub fn stat_or_diff(lines: &[&str], line: usize) -> Option<usize> {
    if let Some(path) = lines.get(line).and_then(|l| stat_path(l)) {
        let header = format!("+++ b/{path}");
        let at = lines.iter().position(|l| *l == header)?;
        return (0..=at).rev().find(|&i| lines[i].starts_with("diff "));
    }
    let file = location(lines, line).map(|l| l.file.to_string_lossy().into_owned());
    let stats: Vec<usize> = (0..lines.len())
        .filter(|&i| stat_path(lines[i]).is_some())
        .collect();
    stats
        .iter()
        .copied()
        .find(|&i| {
            file.as_deref()
                .is_some_and(|f| stat_path(lines[i]) == Some(f))
        })
        .or_else(|| stats.first().copied())
}

/// magit-diff-refine-hunk's word-level difference between a removed and an
/// added line: the changed byte ranges of each.
pub fn refine(old: &str, new: &str) -> (Vec<std::ops::Range<usize>>, Vec<std::ops::Range<usize>>) {
    use similar::{ChangeTag, TextDiff};
    let diff = TextDiff::from_words(old, new);
    let (mut a, mut b) = (vec![], vec![]);
    let (mut i, mut j) = (0, 0);
    let push = |v: &mut Vec<std::ops::Range<usize>>, r: std::ops::Range<usize>| match v.last_mut() {
        Some(last) if last.end == r.start => last.end = r.end,
        _ => v.push(r),
    };
    for change in diff.iter_all_changes() {
        let n = change.value().len();
        match change.tag() {
            ChangeTag::Equal => {
                i += n;
                j += n;
            }
            ChangeTag::Delete => {
                push(&mut a, i..i + n);
                i += n;
            }
            ChangeTag::Insert => {
                push(&mut b, j..j + n);
                j += n;
            }
        }
    }
    // magit-diff-refine-ignore-whitespace (smerge's default, t).
    if super::options::flag("magit-diff-refine-ignore-whitespace", true) {
        a.retain(|r| !old[r.clone()].trim().is_empty());
        b.retain(|r| !new[r.clone()].trim().is_empty());
    }
    (a, b)
}

/// The partner of a removed (added) line in its hunk for refinement: the
/// added (removed) line at the same position of the adjacent run.
pub fn refine_partner(lines: &[&str], l: usize) -> Option<usize> {
    let sign = |i: usize| lines.get(i).and_then(|t| t.chars().next());
    let is = |i: usize, c: char| {
        sign(i) == Some(c) && !lines[i].starts_with("+++") && !lines[i].starts_with("---")
    };
    let me = sign(l)?;
    if me != '-' && me != '+' || !is(l, me) {
        return None;
    }
    let mut start = l;
    while start > 0 && is(start - 1, me) {
        start -= 1;
    }
    let mut end = l + 1;
    while is(end, me) {
        end += 1;
    }
    let other = if me == '-' { '+' } else { '-' };
    let k = l - start;
    if me == '-' {
        (is(end, other) && is(end + k, other)).then_some(end + k)
    } else {
        // The removed run just before this added run.
        let mut s = start;
        while s > 0 && is(s - 1, other) {
            s -= 1;
        }
        (s < start && s + k < start).then_some(s + k)
    }
}
