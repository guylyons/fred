//! `Space j` / `fred DIR`: find-file, vertico style. The query is a path;
//! the part after the last `/` filters the entries of the part before it.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// `dir` as the query starts: `~/…/` under the home directory, ending in `/`.
pub fn show(dir: &Path) -> String {
    let dir = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
    let s = match home().and_then(|h| dir.strip_prefix(h).ok().map(Path::to_path_buf)) {
        Some(rel) if rel.as_os_str().is_empty() => "~".to_string(),
        Some(rel) => format!("~/{}", rel.display()),
        None => dir.display().to_string(),
    };
    if s.ends_with('/') { s } else { format!("{s}/") }
}

/// The query's directory part (through the last `/`) and the name typed after it.
pub fn split(query: &str) -> (&str, &str) {
    match query.rfind('/') {
        Some(i) => (&query[..=i], &query[i + 1..]),
        None => ("", query),
    }
}

/// The directory a query's directory part names (`~` = home, relative = cwd).
pub fn resolve(dir: &str) -> PathBuf {
    let p = match dir.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => home()
            .unwrap_or_default()
            .join(rest.trim_start_matches('/')),
        _ => PathBuf::from(dir),
    };
    std::path::absolute(&p).unwrap_or(p)
}

/// Typing `~/` or a second `/` starts over at home or the root (as Emacs does).
pub fn restart(query: &str) -> Option<String> {
    if query.len() > 2 && query.ends_with("/~/") {
        Some("~/".into())
    } else if query.len() > 1 && query.ends_with("//") {
        Some("/".into())
    } else {
        None
    }
}

/// Backspace right after a `/`: drop the last directory (`~/a/b/` → `~/a/`).
pub fn up(query: &str) -> Option<String> {
    if !query.ends_with('/') {
        return None;
    }
    // From home itself, spell it out so there is a parent to go to.
    let q = if query == "~/" {
        format!("{}/", resolve("~/").display())
    } else {
        query.to_string()
    };
    let t = q.trim_end_matches('/');
    match t.rfind('/') {
        Some(i) => Some(q[..=i].to_string()),
        None if q == "/" => Some(q),
        None => Some(String::new()),
    }
}

/// This directory's entries, directories first, each by name. Honors
/// `.gitignore` and the like; dotfiles only if `hidden`.
pub fn list(dir: &Path, hidden: bool) -> Result<Vec<Entry>, String> {
    std::fs::read_dir(dir).map_err(|e| crate::fileio::err_msg(&e))?;
    let mut out: Vec<Entry> = ignore::WalkBuilder::new(dir)
        .max_depth(Some(1))
        .hidden(!hidden)
        .require_git(false)
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.depth() == 1)
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            // Follow symlinks: a link to a directory opens like one.
            let dir = std::fs::metadata(e.path()).is_ok_and(|m| m.is_dir());
            Some(Entry { name, dir })
        })
        .filter(|e| e.name != ".git")
        .collect();
    out.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.cmp(&b.name)));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_parts() {
        assert_eq!(split("~/src/ma"), ("~/src/", "ma"));
        assert_eq!(split("~/src/"), ("~/src/", ""));
        assert_eq!(split("x"), ("", "x"));
        let home = home().unwrap();
        assert_eq!(resolve("~/"), home);
        assert_eq!(resolve("~/a/"), home.join("a"));
        assert_eq!(resolve("/tmp/"), PathBuf::from("/tmp"));
        assert_eq!(show(&home.join("x")), "~/x/");
        assert_eq!(show(Path::new("/")), "/");
    }

    #[test]
    fn up_and_restart() {
        assert_eq!(up("~/a/b/").as_deref(), Some("~/a/"));
        assert_eq!(up("/a/").as_deref(), Some("/"));
        assert_eq!(up("/").as_deref(), Some("/"));
        assert_eq!(up("~/a/b"), None);
        // From home itself, up goes to its parent, spelled out.
        let parent = show(home().unwrap().parent().unwrap());
        assert_eq!(up("~/").as_deref(), Some(parent.as_str()));
        assert_eq!(restart("~/a/~/").as_deref(), Some("~/"));
        assert_eq!(restart("~/a//").as_deref(), Some("/"));
        assert_eq!(restart("~/a/"), None);
    }

    #[test]
    fn lists_dirs_first() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("zdir")).unwrap();
        std::fs::write(d.path().join("a.txt"), "").unwrap();
        std::fs::write(d.path().join(".hidden"), "").unwrap();
        std::fs::write(d.path().join(".gitignore"), "skipped.log\n").unwrap();
        std::fs::write(d.path().join("skipped.log"), "").unwrap();
        let names = |hidden| -> Vec<(String, bool)> {
            list(d.path(), hidden)
                .unwrap()
                .into_iter()
                .map(|e| (e.name, e.dir))
                .collect()
        };
        assert_eq!(
            names(false),
            [("zdir".into(), true), ("a.txt".into(), false)]
        );
        assert_eq!(
            names(true),
            [
                ("zdir".into(), true),
                (".gitignore".into(), false),
                (".hidden".into(), false),
                ("a.txt".into(), false)
            ]
        );
        assert!(list(&d.path().join("missing"), false).is_err());
    }
}
