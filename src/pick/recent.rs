//! Recently opened files, newest first, and where the cursor was in each:
//! `line<TAB>col<TAB>path` per line (or just a path) in
//! `~/.local/state/fred/recent`. Errors are ignored; losing the list costs
//! nothing.

use std::path::{Path, PathBuf};

pub const MAX: usize = 200;

/// A file and the cursor's (line, byte in line) when it was last left.
pub type Entry = (PathBuf, Option<(usize, usize)>);

pub fn entries(file: &Path) -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return vec![];
    };
    text.lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let mut f = l.splitn(3, '\t');
            match (f.next(), f.next(), f.next()) {
                (Some(a), Some(b), Some(p)) => match (a.parse(), b.parse()) {
                    (Ok(a), Ok(b)) => (PathBuf::from(p), Some((a, b))),
                    _ => (PathBuf::from(l), None),
                },
                _ => (PathBuf::from(l), None),
            }
        })
        .collect()
}

pub fn load(file: &Path) -> Vec<PathBuf> {
    entries(file).into_iter().map(|e| e.0).collect()
}

/// Where the cursor was when `path` was last left.
pub fn position(file: &Path, path: &Path) -> Option<(usize, usize)> {
    let path = std::path::absolute(path).ok()?;
    entries(file).into_iter().find(|e| e.0 == path)?.1
}

/// Move `path` to the front of the list in `file`, with `pos` (or the
/// position it had).
pub fn record(file: &Path, path: &Path, pos: Option<(usize, usize)>) {
    let Ok(path) = std::path::absolute(path) else {
        return;
    };
    let mut list = entries(file);
    let old = list
        .iter()
        .position(|e| e.0 == path)
        .map(|i| list.remove(i));
    list.insert(0, (path, pos.or(old.and_then(|e| e.1))));
    list.truncate(MAX);
    let text: String = list
        .iter()
        .filter_map(|(p, pos)| Some((p.to_str()?, pos)))
        .filter(|(p, _)| !p.contains('\n'))
        .map(|(p, pos)| match pos {
            Some((l, c)) => format!("{l}\t{c}\t{p}\n"),
            None => format!("{p}\n"),
        })
        .collect();
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Write and rename, so two freds never leave a half-written list.
    let tmp = file.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, file).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_dedup_and_trim() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("state/recent");
        assert!(load(&f).is_empty());
        record(&f, Path::new("/a"), None);
        record(&f, Path::new("/b"), None);
        record(&f, Path::new("/a"), None);
        assert_eq!(load(&f), [PathBuf::from("/a"), PathBuf::from("/b")]);
        for i in 0..300 {
            record(&f, Path::new(&format!("/f{i}")), None);
        }
        let l = load(&f);
        assert_eq!(l.len(), MAX);
        assert_eq!(l[0], PathBuf::from("/f299"));
    }

    #[test]
    fn remembers_positions() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("recent");
        record(&f, Path::new("/a"), Some((41, 7)));
        record(&f, Path::new("/b\twith tab"), Some((1, 0)));
        // Opening again (no position) keeps the old one.
        record(&f, Path::new("/a"), None);
        assert_eq!(position(&f, Path::new("/a")), Some((41, 7)));
        assert_eq!(position(&f, Path::new("/b\twith tab")), Some((1, 0)));
        assert_eq!(position(&f, Path::new("/c")), None);
        // The old format, a bare path per line, still reads.
        std::fs::write(&f, "/old\n").unwrap();
        assert_eq!(entries(&f), [(PathBuf::from("/old"), None)]);
    }

    #[test]
    fn unwritable_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        // The state "directory" is a file: nothing can be written under it.
        std::fs::write(dir.path().join("state"), "").unwrap();
        let f = dir.path().join("state/recent");
        record(&f, Path::new("/a"), None);
        assert!(load(&f).is_empty());
    }
}
