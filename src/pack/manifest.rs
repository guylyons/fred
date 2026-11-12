//! `package.toml`, and a package's entry in `config.toml`.

use serde::Deserialize;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    pub grammar: Option<Grammar>,
    #[serde(default)]
    pub theme: Vec<ThemeFile>,
    pub scheme: Option<Scheme>,
}

/// A tree-sitter grammar, compiled from `source` at `rev`. Its queries are
/// the package's `queries/{highlights,injections,folds}.scm`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grammar {
    /// The library exports `tree_sitter_<name>`.
    pub name: String,
    /// A git URL (or a local repository's path).
    pub source: String,
    pub rev: String,
    /// The directory in `source` holding `parser.c` and maybe `scanner.c`.
    #[serde(default = "src")]
    pub dir: String,
    /// Shown in the status line.
    pub status_name: String,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub file_names: Vec<String>,
    /// Interpreter names of a `#!` first line.
    #[serde(default)]
    pub shebangs: Vec<String>,
    /// Names an injection query may use for it, lowercase.
    #[serde(default)]
    pub injection_names: Vec<String>,
    /// Installed packages whose queries go before this one's.
    #[serde(default)]
    pub inherits: Vec<String>,
}

fn src() -> String {
    "src".into()
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeFile {
    pub file: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scheme {
    pub load: Vec<String>,
}

/// `rel` names something inside its directory: no `..`, not absolute.
pub fn inside(rel: &str) -> Result<&Path, String> {
    let p = Path::new(rel);
    if !rel.is_empty() && p.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir)) {
        Ok(p)
    } else {
        Err(format!("path leaves the package: {rel:?}"))
    }
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, String> {
        let m: Manifest = toml::from_str(text).map_err(|e| e.message().to_string())?;
        // The name is a directory, and the grammar's a C symbol.
        let word = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c));
        if !word(&m.name) {
            return Err(format!("bad package name: {:?}", m.name));
        }
        if let Some(g) = &m.grammar {
            if !word(&g.name) || g.name.contains('-') {
                return Err(format!("bad grammar name: {:?}", g.name));
            }
            inside(&g.dir)?;
        }
        let files = m.theme.iter().map(|t| &t.file);
        for f in files.chain(m.scheme.iter().flat_map(|s| &s.load)) {
            inside(f)?;
        }
        Ok(m)
    }

    /// `dir/package.toml`.
    pub fn load(dir: &Path) -> Result<Manifest, String> {
        let text = std::fs::read_to_string(dir.join("package.toml")).map_err(|e| format!("package.toml: {e}"))?;
        Manifest::parse(&text)
    }
}

/// `[[package]]` in config.toml: a git URL or a local directory, and maybe
/// the package's directory inside it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageEntry {
    pub git: Option<String>,
    pub local: Option<PathBuf>,
    pub path: Option<String>,
}

impl PackageEntry {
    /// How messages name the entry before its manifest is read.
    pub fn label(&self) -> String {
        let from = self.git.clone().or(self.local.as_ref().map(|p| p.display().to_string()));
        match (&from, &self.path) {
            (Some(f), Some(p)) => format!("{f} {p}"),
            (Some(f), None) => f.clone(),
            _ => "[[package]]".into(),
        }
    }

    pub fn check(&self) -> Result<(), String> {
        if self.git.is_some() == self.local.is_some() {
            return Err(format!("{}: give one of `git` and `local`", self.label()));
        }
        self.path.as_deref().map_or(Ok(()), |p| inside(p).map(|_| ()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"
name = "lang-rust"
[grammar]
name = "rust"
source = "https://example.com/tree-sitter-rust"
rev = "abc"
status_name = "Rust"
extensions = ["rs"]
injection_names = ["rust", "rs"]
[[theme]]
file = "themes/x.tmTheme"
[scheme]
load = ["init.scm"]
"#;

    #[test]
    fn full_manifest_parses() {
        let m = Manifest::parse(FULL).unwrap();
        let g = m.grammar.unwrap();
        assert_eq!((m.name.as_str(), g.name.as_str(), g.dir.as_str()), ("lang-rust", "rust", "src"));
        assert_eq!((g.extensions, m.theme.len(), m.scheme.unwrap().load), (vec!["rs".to_string()], 1, vec!["init.scm".to_string()]));
    }

    #[test]
    fn unknown_keys_and_bad_names_are_errors() {
        assert!(Manifest::parse("name = \"a\"\nnmae = 1").is_err());
        assert!(Manifest::parse("name = \"../a\"").is_err());
        assert!(Manifest::parse(&FULL.replace("\"rust\"\nsource", "\"ru;st\"\nsource")).is_err());
    }

    #[test]
    fn manifest_paths_cannot_escape() {
        for (from, to) in [
            ("themes/x.tmTheme", "../x.tmTheme"),
            ("themes/x.tmTheme", "/etc/passwd"),
            ("init.scm", "a/../../b.scm"),
            ("rev = \"abc\"", "rev = \"abc\"\ndir = \"../src\""),
        ] {
            assert!(Manifest::parse(&FULL.replace(from, to)).is_err(), "{to}");
        }
    }

    #[test]
    fn an_entry_is_git_or_local() {
        let e = |git: Option<&str>, local: Option<&str>, path: Option<&str>| PackageEntry {
            git: git.map(String::from),
            local: local.map(PathBuf::from),
            path: path.map(String::from),
        };
        assert!(e(Some("u"), None, Some("lang/rust")).check().is_ok());
        assert!(e(None, Some("/p"), None).check().is_ok());
        assert!(e(Some("u"), Some("/p"), None).check().unwrap_err().contains("one of"));
        assert!(e(None, None, None).check().is_err());
        assert!(e(Some("u"), None, Some("../x")).check().is_err());
    }
}
