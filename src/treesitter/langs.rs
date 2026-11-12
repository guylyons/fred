//! The bundled grammars and their queries.

use std::path::Path;
use std::sync::OnceLock;
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
    pub language: fn() -> Language,
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
        language,
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

/// File name, then extension, then a `#!` first line.
pub fn for_file(path: Option<&Path>, first_line: &str) -> Option<&'static LangSpec> {
    let name = path.and_then(|p| p.file_name()).and_then(|n| n.to_str());
    let ext = path.and_then(|p| p.extension()).and_then(|e| e.to_str());
    let by = |f: &dyn Fn(&LangSpec) -> bool| LANGS.iter().find(|s| f(s));
    name.and_then(|n| by(&|s| s.file_names.contains(&n)))
        .or_else(|| ext.and_then(|e| by(&|s| s.extensions.iter().any(|x| x.eq_ignore_ascii_case(e)))))
        .or_else(|| {
            // "#!/usr/bin/env -S python3 -u" → "python3"
            let mut words = first_line.strip_prefix("#!")?.split_whitespace();
            let mut prog = words.next()?.rsplit('/').next()?;
            if prog == "env" {
                prog = words.find(|w| !w.starts_with('-'))?;
            }
            by(&|s| s.shebangs.contains(&prog))
        })
}

/// Case-insensitive.
pub fn by_injection_name(name: &str) -> Option<&'static LangSpec> {
    LANGS
        .iter()
        .find(|s| s.injection_names.iter().any(|n| n.eq_ignore_ascii_case(name)))
}

/// Queries compiled once per language.
pub fn compiled(spec: &'static LangSpec) -> &'static Compiled {
    static CELLS: OnceLock<Vec<OnceLock<Compiled>>> = OnceLock::new();
    let cells = CELLS.get_or_init(|| LANGS.iter().map(|_| OnceLock::new()).collect());
    let i = LANGS
        .iter()
        .position(|s| std::ptr::eq(s, spec))
        .expect("a LangSpec from LANGS");
    cells[i].get_or_init(|| {
        let language = (spec.language)();
        let query = |parts: &[&str], what: &str| {
            Query::new(&language, &parts.join("\n"))
                .unwrap_or_else(|err| panic!("{} ({what}): {err}", spec.name))
        };
        let optional = |parts: &[&str], what: &str| (!parts.is_empty()).then(|| query(parts, what));
        Compiled {
            highlights: query(spec.highlights, "highlights"),
            injections: optional(spec.injections, "injections"),
            folds: optional(spec.folds, "folds"),
            language,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
