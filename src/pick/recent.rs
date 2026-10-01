//! Recently opened files, newest first: one absolute path per line in
//! `~/.local/state/fred/recent`. Errors are ignored; losing the list costs
//! nothing.

use std::path::{Path, PathBuf};

pub const MAX: usize = 200;

pub fn load(file: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(file).map_or(vec![], |t| {
        t.lines()
            .filter(|l| !l.is_empty())
            .map(PathBuf::from)
            .collect()
    })
}

/// Move `path` to the front of the list in `file`.
pub fn record(file: &Path, path: &Path) {
    let Ok(path) = std::path::absolute(path) else {
        return;
    };
    let mut list = load(file);
    list.retain(|p| *p != path);
    list.insert(0, path);
    list.truncate(MAX);
    let text: String = list
        .iter()
        .filter_map(|p| p.to_str())
        .filter(|p| !p.contains('\n'))
        .map(|p| format!("{p}\n"))
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
        record(&f, Path::new("/a"));
        record(&f, Path::new("/b"));
        record(&f, Path::new("/a"));
        assert_eq!(load(&f), [PathBuf::from("/a"), PathBuf::from("/b")]);
        for i in 0..300 {
            record(&f, Path::new(&format!("/f{i}")));
        }
        let l = load(&f);
        assert_eq!(l.len(), MAX);
        assert_eq!(l[0], PathBuf::from("/f299"));
    }

    #[test]
    fn unwritable_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        // The state "directory" is a file: nothing can be written under it.
        std::fs::write(dir.path().join("state"), "").unwrap();
        let f = dir.path().join("state/recent");
        record(&f, Path::new("/a"));
        assert!(load(&f).is_empty());
    }
}
