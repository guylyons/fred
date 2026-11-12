//! Packages: directories of grammars, queries, themes and scripts that
//! live outside the binary (`~/.local/share/fred/packages/<name>/`).

pub mod installed;
pub mod manifest;
pub mod sync;

use std::path::{Path, PathBuf};

/// Where packages are installed: `$XDG_DATA_HOME/fred/packages`, else
/// `~/.local/share/fred/packages`.
pub fn root() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".local/share")))?;
    Some(data.join("fred/packages"))
}

/// At startup: tell the language table about the installed packages.
/// Nothing is loaded or compiled until a file needs it.
pub fn load() {
    let Some(root) = root() else {
        return;
    };
    let (ok, bad) = installed::installed(&root);
    for (name, err) in bad {
        crate::treesitter::langs::report(format!("{name}: {err}"));
    }
    crate::treesitter::langs::add_packages(&ok);
}

/// `fred pack ARGS` and `:pack ARGS`: sync, update [NAME] or list. The
/// lines to show, and whether all went well.
pub fn command(args: &str) -> (Vec<String>, bool) {
    let Some(root) = root() else {
        return (vec!["pack: no home directory".into()], false);
    };
    let (cfg, err) = crate::config::Config::load();
    if let Some(err) = err {
        return (vec![err], false);
    }
    let lock = crate::config::config_path().with_file_name("packages.lock");
    run(args, &cfg.package, &lock, &root)
}

fn run(args: &str, entries: &[manifest::PackageEntry], lock_path: &Path, root: &Path) -> (Vec<String>, bool) {
    let mut words = args.split_whitespace();
    let mode = match (words.next(), words.next(), words.next()) {
        (Some("sync"), None, _) => sync::Mode::Sync,
        (Some("update"), name, None) => sync::Mode::Update(name.map(String::from)),
        (Some("list"), None, _) => return list(lock_path, root),
        _ => return (vec!["usage: pack sync | update [NAME] | list".into()], false),
    };
    let mut lock = sync::Lock::load(lock_path);
    let reports = sync::sync(entries, &mut lock, root, mode);
    let mut ok = reports.iter().all(|r| r.result.is_ok());
    let mut lines: Vec<String> = reports
        .iter()
        .map(|r| match &r.result {
            Ok(did) => format!("{}: {did}", r.name),
            Err(e) => format!("{}: error: {e}", r.name),
        })
        .collect();
    if let Err(e) = lock.save(lock_path) {
        lines.push(e);
        ok = false;
    }
    if lines.is_empty() {
        lines.push("no packages in config.toml".into());
    }
    (lines, ok)
}

fn list(lock_path: &Path, root: &Path) -> (Vec<String>, bool) {
    let lock = sync::Lock::load(lock_path);
    let (ok, bad) = installed::installed(root);
    let errors = crate::treesitter::langs::errors();
    let mut lines: Vec<String> = ok
        .iter()
        .map(|p| {
            let name = &p.manifest.name;
            let commit = lock.packages.iter().find(|l| &l.name == name).and_then(|l| l.commit.as_deref());
            let at = commit.map_or("local", |c| &c[..c.len().min(8)]);
            match errors.iter().find(|e| e.starts_with(&format!("{name}: "))) {
                Some(e) => format!("{name} {at} error: {}", &e[name.len() + 2..]),
                None => format!("{name} {at}"),
            }
        })
        .collect();
    lines.extend(bad.iter().map(|(name, e)| format!("{name} error: {e}")));
    if lines.is_empty() {
        lines.push("no packages installed".into());
    }
    (lines, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_command_syncs_lists_and_rejects_nonsense() {
        let w = sync::tests::world();
        let lock = w.dir.path().join("packages.lock");
        let entries = [w.entry.clone()];
        let (lines, ok) = run("sync", &entries, &lock, &w.root);
        assert!(ok && lines[0].starts_with("lang-json: installed "), "{lines:?}");
        assert!(lock.exists());
        std::fs::create_dir_all(w.root.join("broken")).unwrap();
        let (lines, ok) = run("list", &entries, &lock, &w.root);
        assert!(ok && lines.len() == 2, "{lines:?}");
        assert!(lines[0].starts_with("lang-json ") && lines[0].len() == "lang-json ".len() + 8, "{lines:?}");
        assert!(lines[1].starts_with("broken error: package.toml: "), "{lines:?}");
        for bad in ["", "sync x", "update a b", "frobnicate"] {
            let (lines, ok) = run(bad, &entries, &lock, &w.root);
            assert!(!ok && lines[0].starts_with("usage: pack "), "{bad}");
        }
        assert_eq!(run("sync", &[], &lock, &w.root).0, ["lang-json: removed"]);
        assert_eq!(run("sync", &[], &lock, &w.root).0, ["no packages in config.toml"]);
    }
}
