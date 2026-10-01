//! File path completion.

use std::path::{Path, PathBuf};

fn expand(dir: &str, cwd: &Path) -> PathBuf {
    if let Some(rest) = dir.strip_prefix('~') {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        return home.join(rest.trim_start_matches('/'));
    }
    if dir.is_empty() {
        return cwd.to_path_buf();
    }
    let p = Path::new(dir);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    }
}

/// Paths starting with `prefix`; directories end in `/`.
pub fn complete(prefix: &str, cwd: &Path) -> Vec<String> {
    let (dir, name) = match prefix.rfind('/') {
        Some(i) => (&prefix[..=i], &prefix[i + 1..]),
        None if prefix == "~" => return vec!["~/".into()],
        None => ("", prefix),
    };
    let Ok(entries) = std::fs::read_dir(expand(dir, cwd)) else {
        return vec![];
    };
    let mut out: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            if !n.starts_with(name) || (n.starts_with('.') && !name.starts_with('.')) {
                return None;
            }
            let is_dir = e.path().is_dir();
            Some(format!("{dir}{n}{}", if is_dir { "/" } else { "" }))
        })
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn completes_paths() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("src")).unwrap();
        fs::write(d.path().join("Cargo.toml"), "").unwrap();
        fs::write(d.path().join(".hidden"), "").unwrap();
        assert_eq!(complete("./C", d.path()), vec!["./Cargo.toml"]);
        assert_eq!(complete("./s", d.path()), vec!["./src/"]);
        assert_eq!(complete("./", d.path()), vec!["./Cargo.toml", "./src/"]);
        assert_eq!(complete("./.h", d.path()), vec!["./.hidden"]);
        assert_eq!(complete("s", d.path()), vec!["src/"]);
        assert!(complete("./nope/x", d.path()).is_empty());
    }
}
