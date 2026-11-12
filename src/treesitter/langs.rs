//! The languages: bundled grammars and installed packages', with their queries.

use crate::pack::installed::Installed;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tree_sitter::{Language, Query};

pub struct LangSpec {
    /// Shown in the status line; equals syntect's name for the same file when syntect knows the language.
    pub name: &'static str,
    /// Names an injection query may use for it ("rust", "rs", "js", "javascript", …), lowercase.
    pub injection_names: &'static [&'static str],
    pub extensions: &'static [&'static str],
    /// "Makefile", "Dockerfile", …
    pub file_names: &'static [&'static str],
    /// Interpreter names: "python3", "bash", "node", …
    pub shebangs: &'static [&'static str],
    pub language: Source,
    /// Concatenated in order (TypeScript = JavaScript's + TypeScript's; C++ = C's + C++'s).
    pub highlights: &'static [&'static str],
    pub injections: &'static [&'static str],
    /// Fold query sources, concatenated like `highlights`.
    pub folds: &'static [&'static str],
    /// When two patterns capture the same node, the earlier one colours it.
    /// Most queries are written for the later one to win (generic
    /// `(identifier) @variable` first); Go's is written the other way round.
    pub earlier_wins: bool,
}

/// Where a language's grammar comes from.
#[derive(Clone, Copy)]
pub enum Source {
    Bundled(fn() -> Language),
    /// A package's compiled grammar and the directories of its query
    /// files, read in order.
    Library {
        package: &'static str,
        path: &'static Path,
        symbol: &'static str,
        queries: &'static [PathBuf],
    },
}

pub struct Compiled {
    pub language: Language,
    pub highlights: Query,
    pub injections: Option<Query>,
    pub folds: Option<Query>,
}

const fn lang(
    name: &'static str,
    language: fn() -> Language,
    highlights: &'static [&'static str],
) -> LangSpec {
    LangSpec {
        name,
        injection_names: &[],
        extensions: &[],
        file_names: &[],
        shebangs: &[],
        language: Source::Bundled(language),
        highlights,
        injections: &[],
        folds: &[],
        earlier_wins: false,
    }
}

const JS: &str = tree_sitter_javascript::HIGHLIGHT_QUERY;
const TS: &str = tree_sitter_typescript::HIGHLIGHTS_QUERY;
const C: &str = tree_sitter_c::HIGHLIGHT_QUERY;

pub static LANGS: &[LangSpec] = &[
    LangSpec {
        injection_names: &["rust", "rs"],
        extensions: &["rs"],
        injections: &[tree_sitter_rust::INJECTIONS_QUERY],
        folds: &[include_str!("../../queries/rust/folds.scm")],
        ..lang("Rust", || tree_sitter_rust::LANGUAGE.into(), &[tree_sitter_rust::HIGHLIGHTS_QUERY])
    },
    LangSpec {
        injection_names: &["typescript", "ts"],
        extensions: &["ts", "mts", "cts"],
        shebangs: &["ts-node", "tsx", "deno", "bun"],
        injections: &[tree_sitter_javascript::INJECTIONS_QUERY],
        folds: &[include_str!("../../queries/ecma/folds.scm"), include_str!("../../queries/typescript/folds.scm")],
        ..lang("TypeScript", || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), &[JS, TS])
    },
    LangSpec {
        injection_names: &["tsx"],
        extensions: &["tsx"],
        injections: &[tree_sitter_javascript::INJECTIONS_QUERY],
        folds: &[include_str!("../../queries/ecma/folds.scm"), include_str!("../../queries/typescript/folds.scm"), include_str!("../../queries/jsx/folds.scm")],
        ..lang(
            "TypeScriptReact",
            || tree_sitter_typescript::LANGUAGE_TSX.into(),
            &[JS, tree_sitter_javascript::JSX_HIGHLIGHT_QUERY, TS],
        )
    },
    LangSpec {
        injection_names: &["javascript", "js", "jsx", "node"],
        extensions: &["js", "mjs", "cjs", "jsx"],
        shebangs: &["node", "nodejs"],
        injections: &[tree_sitter_javascript::INJECTIONS_QUERY],
        folds: &[include_str!("../../queries/ecma/folds.scm"), include_str!("../../queries/jsx/folds.scm")],
        ..lang(
            "JavaScript (Babel)",
            || tree_sitter_javascript::LANGUAGE.into(),
            &[JS, tree_sitter_javascript::JSX_HIGHLIGHT_QUERY],
        )
    },
    LangSpec {
        injection_names: &["python", "py", "python3"],
        extensions: &["py", "pyi", "pyw"],
        shebangs: &["python", "python2", "python3"],
        folds: &[include_str!("../../queries/python/folds.scm")],
        ..lang("Python", || tree_sitter_python::LANGUAGE.into(), &[tree_sitter_python::HIGHLIGHTS_QUERY])
    },
    LangSpec {
        injection_names: &["go", "golang"],
        extensions: &["go"],
        earlier_wins: true,
        folds: &[include_str!("../../queries/go/folds.scm")],
        ..lang("Go", || tree_sitter_go::LANGUAGE.into(), &[tree_sitter_go::HIGHLIGHTS_QUERY])
    },
    LangSpec {
        injection_names: &["c", "h"],
        extensions: &["c", "h"],
        folds: &[include_str!("../../queries/c/folds.scm")],
        ..lang("C", || tree_sitter_c::LANGUAGE.into(), &[C])
    },
    LangSpec {
        injection_names: &["cpp", "c++", "cxx", "cc", "hpp"],
        extensions: &["cpp", "cc", "cxx", "c++", "hpp", "hh", "hxx", "h++", "ipp", "inl"],
        injections: &[include_str!("../../queries/cpp/injections.scm")],
        folds: &[include_str!("../../queries/c/folds.scm"), include_str!("../../queries/cpp/folds.scm")],
        ..lang("C++", || tree_sitter_cpp::LANGUAGE.into(), &[C, tree_sitter_cpp::HIGHLIGHT_QUERY])
    },
    LangSpec {
        injection_names: &["swift"],
        extensions: &["swift"],
        injections: &[tree_sitter_swift::INJECTIONS_QUERY],
        folds: &[include_str!("../../queries/swift/folds.scm")],
        ..lang("Swift", || tree_sitter_swift::LANGUAGE.into(), &[tree_sitter_swift::HIGHLIGHTS_QUERY])
    },
    LangSpec {
        injection_names: &["lua"],
        extensions: &["lua"],
        shebangs: &["lua", "luajit"],
        injections: &[tree_sitter_lua::INJECTIONS_QUERY],
        folds: &[include_str!("../../queries/lua/folds.scm")],
        ..lang("Lua", || tree_sitter_lua::LANGUAGE.into(), &[tree_sitter_lua::HIGHLIGHTS_QUERY])
    },
    LangSpec {
        injection_names: &["ruby", "rb"],
        extensions: &["rb", "rake", "gemspec", "ru"],
        file_names: &["Rakefile", "Gemfile"],
        shebangs: &["ruby"],
        folds: &[include_str!("../../queries/ruby/folds.scm")],
        ..lang("Ruby", || tree_sitter_ruby::LANGUAGE.into(), &[tree_sitter_ruby::HIGHLIGHTS_QUERY])
    },
    LangSpec {
        injection_names: &["java"],
        extensions: &["java"],
        folds: &[include_str!("../../queries/java/folds.scm")],
        ..lang("Java", || tree_sitter_java::LANGUAGE.into(), &[tree_sitter_java::HIGHLIGHTS_QUERY])
    },
    LangSpec {
        injection_names: &["bash", "sh", "shell", "zsh"],
        extensions: &["sh", "bash", "zsh"],
        file_names: &[".bashrc", ".bash_profile", ".zshrc", ".zshenv", ".profile"],
        shebangs: &["sh", "bash", "zsh", "dash", "ksh"],
        folds: &[include_str!("../../queries/bash/folds.scm")],
        ..lang(
            "Bourne Again Shell (bash)",
            || tree_sitter_bash::LANGUAGE.into(),
            &[tree_sitter_bash::HIGHLIGHT_QUERY],
        )
    },
    LangSpec {
        injection_names: &["html", "htm"],
        extensions: &["html", "htm", "xhtml"],
        injections: &[tree_sitter_html::INJECTIONS_QUERY],
        folds: &[include_str!("../../queries/html/folds.scm")],
        ..lang("HTML", || tree_sitter_html::LANGUAGE.into(), &[tree_sitter_html::HIGHLIGHTS_QUERY])
    },
    LangSpec {
        injection_names: &["css"],
        extensions: &["css"],
        folds: &[include_str!("../../queries/css/folds.scm")],
        ..lang("CSS", || tree_sitter_css::LANGUAGE.into(), &[tree_sitter_css::HIGHLIGHTS_QUERY])
    },
    LangSpec {
        injection_names: &["json", "jsonc"],
        extensions: &["json", "jsonc"],
        folds: &[include_str!("../../queries/json/folds.scm")],
        ..lang("JSON", || tree_sitter_json::LANGUAGE.into(), &[tree_sitter_json::HIGHLIGHTS_QUERY])
    },
    LangSpec {
        injection_names: &["toml"],
        extensions: &["toml"],
        file_names: &["Cargo.lock"],
        folds: &[include_str!("../../queries/toml/folds.scm")],
        ..lang("TOML", || tree_sitter_toml_ng::LANGUAGE.into(), &[tree_sitter_toml_ng::HIGHLIGHTS_QUERY])
    },
    LangSpec {
        injection_names: &["yaml", "yml"],
        extensions: &["yaml", "yml"],
        folds: &[include_str!("../../queries/yaml/folds.scm")],
        ..lang("YAML", || tree_sitter_yaml::LANGUAGE.into(), &[tree_sitter_yaml::HIGHLIGHTS_QUERY])
    },
    LangSpec {
        injection_names: &["markdown", "md"],
        extensions: &["md", "markdown", "mdown", "mkd"],
        injections: &[tree_sitter_md::INJECTION_QUERY_BLOCK],
        folds: &[include_str!("../../queries/markdown/folds.scm")],
        ..lang(
            "Markdown",
            || tree_sitter_md::LANGUAGE.into(),
            // A fence's language name is a constant to syntect's Markdown too.
            &[tree_sitter_md::HIGHLIGHT_QUERY_BLOCK, "(info_string (language) @constant)"],
        )
    },
    LangSpec {
        injection_names: &["markdown_inline"],
        injections: &[tree_sitter_md::INJECTION_QUERY_INLINE],
        ..lang(
            "Markdown Inline",
            || tree_sitter_md::INLINE_LANGUAGE.into(),
            &[tree_sitter_md::HIGHLIGHT_QUERY_INLINE],
        )
    },
    LangSpec {
        injection_names: &["elixir", "ex", "exs"],
        extensions: &["ex", "exs"],
        shebangs: &["elixir"],
        injections: &[tree_sitter_elixir::INJECTIONS_QUERY],
        folds: &[include_str!("../../queries/elixir/folds.scm")],
        ..lang("Elixir", || tree_sitter_elixir::LANGUAGE.into(), &[tree_sitter_elixir::HIGHLIGHTS_QUERY])
    },
    LangSpec {
        injection_names: &["php"],
        extensions: &["php", "phtml"],
        shebangs: &["php"],
        injections: &[tree_sitter_php::INJECTIONS_QUERY],
        folds: &[include_str!("../../queries/php_only/folds.scm")],
        ..lang("PHP", || tree_sitter_php::LANGUAGE_PHP.into(), &[tree_sitter_php::HIGHLIGHTS_QUERY])
    },
    LangSpec {
        injection_names: &["sql"],
        extensions: &["sql"],
        folds: &[include_str!("../../queries/sql/folds.scm")],
        ..lang("SQL", || tree_sitter_sequel::LANGUAGE.into(), &[tree_sitter_sequel::HIGHLIGHTS_QUERY])
    },
    LangSpec {
        injection_names: &["zig"],
        extensions: &["zig", "zon"],
        injections: &[tree_sitter_zig::INJECTIONS_QUERY],
        folds: &[include_str!("../../queries/zig/folds.scm")],
        ..lang("Zig", || tree_sitter_zig::LANGUAGE.into(), &[tree_sitter_zig::HIGHLIGHTS_QUERY])
    },
    LangSpec {
        injection_names: &["make", "makefile"],
        extensions: &["mk", "mak"],
        file_names: &["Makefile", "makefile", "GNUmakefile"],
        folds: &[include_str!("../../queries/make/folds.scm")],
        ..lang("Makefile", || tree_sitter_make::LANGUAGE.into(), &[include_str!("../../queries/make/highlights.scm")])
    },
    LangSpec {
        injection_names: &["dockerfile", "docker"],
        extensions: &["dockerfile"],
        file_names: &["Dockerfile", "Containerfile"],
        ..lang(
            "Dockerfile",
            tree_sitter_dockerfile_updated::language,
            &[include_str!("../../queries/dockerfile/highlights.scm")],
        )
    },
];

/// The grammars of installed packages, set once at startup. A package's
/// language comes before a bundled one for the same file.
static PACKAGES: OnceLock<Vec<LangSpec>> = OnceLock::new();
/// Why packages aren't used, and how many of them [`new_errors`] has given.
static ERRORS: Mutex<(Vec<String>, usize)> = Mutex::new((vec![], 0));

/// A package can't be used: "name: why".
pub fn report(error: String) {
    ERRORS.lock().unwrap().0.push(error);
}

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

fn leak_all(v: &[String]) -> &'static [&'static str] {
    Box::leak(v.iter().map(|s| leak(s)).collect::<Vec<_>>().into_boxed_slice())
}

/// Add the installed packages' grammars. Call once, before the first file
/// is opened; nothing is loaded or compiled until a file needs it.
pub fn add_packages(installed: &[Installed]) {
    let specs = installed
        .iter()
        .filter_map(|p| {
            let g = p.manifest.grammar.as_ref()?;
            // Inherited packages' queries first, then its own.
            let mut dirs = vec![];
            for name in &g.inherits {
                match installed.iter().find(|i| &i.manifest.name == name) {
                    Some(i) => dirs.push(i.dir.join("queries")),
                    None => {
                        report(format!("{}: inherits {name}, which is not installed", p.manifest.name));
                        return None;
                    }
                }
            }
            dirs.push(p.dir.join("queries"));
            Some(LangSpec {
                injection_names: leak_all(&g.injection_names),
                extensions: leak_all(&g.extensions),
                file_names: leak_all(&g.file_names),
                shebangs: leak_all(&g.shebangs),
                language: Source::Library {
                    package: leak(&p.manifest.name),
                    path: Box::leak(p.dir.join(crate::pack::sync::library_name()).into_boxed_path()),
                    symbol: leak(&format!("tree_sitter_{}", g.name)),
                    queries: Box::leak(dirs.into_boxed_slice()),
                },
                ..lang(leak(&g.status_name), || unreachable!(), &[])
            })
        })
        .collect();
    let _ = PACKAGES.set(specs);
}

/// Package errors not given before (the message line shows each once).
pub fn new_errors() -> Vec<String> {
    let mut e = ERRORS.lock().unwrap();
    let len = e.0.len();
    let from = std::mem::replace(&mut e.1, len);
    e.0[from..].to_vec()
}

/// Every package error so far (`:pack list`).
pub fn errors() -> Vec<String> {
    ERRORS.lock().unwrap().0.clone()
}

/// Every language: the packages', then the bundled ones.
pub fn all() -> impl Iterator<Item = &'static LangSpec> {
    PACKAGES.get().into_iter().flatten().chain(LANGS)
}

/// The first language `f` picks whose grammar loads.
fn find(f: &dyn Fn(&LangSpec) -> bool) -> Option<&'static LangSpec> {
    all().find(|s| f(s) && try_compiled(s).is_ok())
}

/// File name, then extension, then a `#!` first line.
pub fn for_file(path: Option<&Path>, first_line: &str) -> Option<&'static LangSpec> {
    let name = path.and_then(|p| p.file_name()).and_then(|n| n.to_str());
    let ext = path.and_then(|p| p.extension()).and_then(|e| e.to_str());
    name.and_then(|n| find(&|s| s.file_names.contains(&n)))
        .or_else(|| ext.and_then(|e| find(&|s| s.extensions.iter().any(|x| x.eq_ignore_ascii_case(e)))))
        .or_else(|| {
            // "#!/usr/bin/env -S python3 -u" → "python3"
            let mut words = first_line.strip_prefix("#!")?.split_whitespace();
            let mut prog = words.next()?.rsplit('/').next()?;
            if prog == "env" {
                prog = words.find(|w| !w.starts_with('-'))?;
            }
            find(&|s| s.shebangs.contains(&prog))
        })
}

/// Case-insensitive.
pub fn by_injection_name(name: &str) -> Option<&'static LangSpec> {
    find(&|s| s.injection_names.iter().any(|n| n.eq_ignore_ascii_case(name)))
}

/// A package's grammar from its library, and its query files' text.
fn load(path: &Path, symbol: &str, dirs: &[PathBuf]) -> Result<(Language, [String; 3]), String> {
    // The library stays loaded for the life of the process: trees and
    // queries point into it.
    let lib = unsafe { libloading::Library::new(path) }.map_err(|e| format!("{}: {e}", path.display()))?;
    let lib = Box::leak(Box::new(lib));
    let f = unsafe { lib.get::<unsafe extern "C" fn() -> *const ()>(symbol.as_bytes()) }
        .map_err(|_| format!("no {symbol} in {}", path.display()))?;
    let language = Language::new(unsafe { tree_sitter_language::LanguageFn::from_raw(*f) });
    let abi = language.abi_version();
    if !(tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=tree_sitter::LANGUAGE_VERSION).contains(&abi) {
        return Err(format!(
            "grammar ABI {abi}, Fred reads {} to {}",
            tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION,
            tree_sitter::LANGUAGE_VERSION
        ));
    }
    let read = |what: &str| -> String {
        let files = dirs.iter().map(|d| d.join(format!("{what}.scm")));
        files.filter_map(|f| std::fs::read_to_string(f).ok()).collect::<Vec<_>>().join("\n")
    };
    Ok((language, [read("highlights"), read("injections"), read("folds")]))
}

fn compile(spec: &LangSpec) -> Result<Compiled, String> {
    let (language, [highlights, injections, folds]) = match spec.language {
        Source::Bundled(f) => (f(), [spec.highlights, spec.injections, spec.folds].map(|p| p.join("\n"))),
        Source::Library { path, symbol, queries, .. } => load(path, symbol, queries)?,
    };
    let query = |text: &str, what: &str| {
        Query::new(&language, text).map_err(|err| format!("{what} query: {err}"))
    };
    let optional = |text: &str, what: &str| (!text.trim().is_empty()).then(|| query(text, what)).transpose();
    Ok(Compiled {
        highlights: query(&highlights, "highlights")?,
        injections: optional(&injections, "injections")?,
        folds: optional(&folds, "folds")?,
        language,
    })
}

/// The language's grammar and queries, loaded and compiled once. A
/// package's failure is kept, and reported through [`new_errors`].
pub fn try_compiled(spec: &'static LangSpec) -> Result<&'static Compiled, &'static str> {
    static CELLS: Mutex<Option<HashMap<usize, &'static Result<Compiled, String>>>> = Mutex::new(None);
    let key = spec as *const LangSpec as usize;
    let mut cells = CELLS.lock().unwrap();
    let cell = cells.get_or_insert_with(HashMap::new).entry(key).or_insert_with(|| {
        let result = compile(spec);
        if let (Err(e), Source::Library { package, .. }) = (&result, &spec.language) {
            report(format!("{package}: {e}"));
        }
        Box::leak(Box::new(result))
    });
    cell.as_ref().map_err(String::as_str)
}

/// As [`try_compiled`], for a language [`for_file`] or
/// [`by_injection_name`] gave (they skip a package that fails), or a
/// bundled one (a bad bundled query is a bug: tests compile them all).
pub fn compiled(spec: &'static LangSpec) -> &'static Compiled {
    try_compiled(spec).unwrap_or_else(|err| panic!("{}: {err}", spec.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixture JSON grammar as an installed package claiming `exts`,
    /// with `highlights` as its highlight query.
    fn package(dir: &Path, name: &str, c_source: Option<&str>, symbol: &str, exts: &[&str], highlights: &str) -> Installed {
        use crate::pack::sync::{compile, library_name, tests::FIXTURE};
        let pkg = dir.join(name);
        std::fs::create_dir_all(pkg.join("queries")).unwrap();
        let src = match c_source {
            Some(c) => {
                std::fs::create_dir_all(pkg.join("src")).unwrap();
                std::fs::write(pkg.join("src/parser.c"), c).unwrap();
                pkg.join("src")
            }
            None => Path::new(FIXTURE).join("src"),
        };
        compile(&src, &pkg.join(library_name())).unwrap();
        std::fs::write(pkg.join("queries/highlights.scm"), highlights).unwrap();
        std::fs::write(pkg.join("queries/folds.scm"), "[(object) (array)] @fold").unwrap();
        let exts: Vec<String> = exts.iter().map(|e| format!("{e:?}")).collect();
        let manifest = format!(
            "name = {name:?}\n[grammar]\nname = {symbol:?}\nsource = \"x\"\nrev = \"x\"\nstatus_name = {name:?}\nextensions = [{}]\n",
            exts.join(",")
        );
        Installed { manifest: crate::pack::manifest::Manifest::parse(&manifest).unwrap(), dir: pkg }
    }

    // One test: the package table is set once per process.
    #[test]
    fn packages_add_languages_override_bundled_ones_and_fail_alone() {
        let d = tempfile::tempdir().unwrap();
        let abi1 = format!(
            "#include \"{}/src/tree_sitter/parser.h\"\nstatic const TSLanguage l = {{ .version = 1 }};\nconst TSLanguage *tree_sitter_old(void) {{ return &l; }}\n",
            crate::pack::sync::tests::FIXTURE
        );
        add_packages(&[
            package(d.path(), "good", None, "json", &["jsonx", "json"], "(string) @string"),
            package(d.path(), "wrong-abi", Some(&abi1), "old", &["oldabi"], ""),
            package(d.path(), "bad-query", None, "json", &["badq"], "(no_such_node) @string"),
            package(d.path(), "no-symbol", None, "missing", &["nosym"], "(string) @string"),
        ]);
        let f = |p: &str| for_file(Some(Path::new(p)), "").map(|s| s.name);
        // A new extension, and a bundled one the package takes over.
        assert_eq!((f("a.jsonx"), f("a.json")), (Some("good"), Some("good")));
        let c = compiled(for_file(Some(Path::new("a.jsonx")), "").unwrap());
        assert!(c.folds.is_some() && c.highlights.capture_names() == ["string"]);
        // Each broken package is skipped (syntect takes the file) and named once.
        assert_eq!((f("a.oldabi"), f("a.badq"), f("a.nosym"), f("a.oldabi")), (None, None, None, None));
        let errors = super::errors();
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert!(errors[0].starts_with("wrong-abi: grammar ABI 1, Fred reads "), "{errors:?}");
        assert!(errors[1].starts_with("bad-query: highlights query: "), "{errors:?}");
        assert!(errors[2].starts_with("no-symbol: no tree_sitter_missing in "), "{errors:?}");
        assert_eq!(new_errors(), errors, "each is new once");
        assert!(new_errors().is_empty());
        // Bundled languages are still there.
        assert_eq!(f("x.rs"), Some("Rust"));
    }

    #[test]
    fn picks_language_by_name_extension_and_shebang() {
        let f = |p: &str, first: &str| for_file(Some(Path::new(p)), first).map(|s| s.name);
        assert_eq!(f("src/x.rs", ""), Some("Rust"));
        assert_eq!(f("a.tsx", ""), for_file(Some(Path::new("b.tsx")), "").map(|s| s.name));
        assert_ne!(f("a.tsx", ""), f("a.ts", ""));
        assert_eq!(f("Makefile", ""), Some("Makefile"));
        assert_eq!(for_file(None, "#!/usr/bin/env python3").map(|s| s.name), Some("Python"));
        assert_eq!(f("x.zzz-unknown", ""), None);
        assert_eq!(by_injection_name("JS").map(|s| s.name), f("a.js", ""));
    }

    #[test]
    fn every_query_compiles_and_names_match_syntect() {
        let ss = two_face::syntax::extra_newlines();
        for spec in LANGS {
            let c = compiled(spec); // panics with the language name on a bad query
            assert!(c.highlights.capture_names().len() > 0, "{}", spec.name);
            if let Some(ext) = spec.extensions.first() {
                if let Some(syn) = ss.find_syntax_by_extension(ext) {
                    assert_eq!(spec.name, syn.name, "status-line name for .{ext}");
                }
            }
        }
    }
}
