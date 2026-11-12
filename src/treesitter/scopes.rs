//! Capture names → TextMate scopes, so the syntect theme colours tree-sitter's captures.

use crate::highlight::to_style;
use ratatui::style::Style;
use syntect::highlighting::{self as sh, FontStyle, Theme};
use syntect::parsing::Scope;

/// Lookup tries the full name, then drops one trailing component at a time.
/// A value can be a scope stack, outermost first ("markup.heading
/// entity.name.section"), for themes that colour only one of the names, or only inside Markdown.
const TABLE: &[(&str, &str)] = &[
    ("attribute", "entity.other.attribute-name"),
    ("boolean", "constant.language"),
    ("character", "constant.character"),
    ("character.special", "constant.character.escape"),
    ("comment", "comment"),
    ("comment.doc", "comment.block.documentation"),
    ("comment.documentation", "comment.block.documentation"),
    ("conditional", "keyword.control"),
    ("constant", "constant.other"),
    ("constant.builtin", "constant.language"),
    ("constant.macro", "constant.other"),
    ("constructor", "entity.name.type"),
    ("delimiter", "punctuation.separator"),
    ("embedded", "meta.embedded"),
    ("error", "invalid.illegal"),
    ("escape", "constant.character.escape"),
    ("exception", "keyword.control"),
    ("field", "variable.other.member"),
    ("float", "constant.numeric"),
    ("function", "entity.name.function"),
    ("function.builtin", "support.function"),
    ("function.call", "variable.function"),
    ("function.macro", "entity.name.function.macro"),
    ("function.method", "entity.name.function"),
    ("function.special", "entity.name.function.macro"),
    ("import", "keyword.control.import"),
    ("include", "keyword.control.import"),
    ("keyword", "keyword.control"),
    ("keyword.function", "storage.type.function"),
    ("keyword.modifier", "storage.modifier"),
    ("keyword.operator", "keyword.operator"),
    ("keyword.type", "storage.type"),
    ("label", "entity.name.label"),
    ("markup.bold", "text.html.markdown markup.bold"),
    ("markup.heading", "text.html.markdown markup.heading.markdown entity.name.section.markdown"),
    ("markup.italic", "text.html.markdown markup.italic"),
    ("markup.link.url", "text.html.markdown markup.underline.link"),
    ("markup.raw", "text.html.markdown markup.raw"),
    ("method", "entity.name.function"),
    ("method.call", "variable.function"),
    ("module", "entity.name.namespace"),
    ("namespace", "entity.name.namespace"),
    ("number", "constant.numeric"),
    ("operator", "keyword.operator"),
    ("parameter", "variable.parameter"),
    ("preproc", "keyword.control.directive"),
    ("property", "variable.other.member"),
    ("punctuation", "punctuation"),
    ("punctuation.bracket", "punctuation.section"),
    ("punctuation.delimiter", "punctuation.separator"),
    ("punctuation.special", "punctuation.definition"),
    ("repeat", "keyword.control"),
    ("storageclass", "storage.modifier"),
    ("string", "string.quoted"),
    ("string.escape", "constant.character.escape"),
    ("string.regex", "string.regexp"),
    ("string.regexp", "string.regexp"),
    ("string.special", "string.regexp"),
    ("string.special.key", "entity.name.tag"),
    ("string.special.symbol", "constant.other.symbol"),
    ("tag", "entity.name.tag"),
    ("tag.error", "invalid.illegal"),
    ("text.danger", "invalid.illegal"),
    ("text.emphasis", "text.html.markdown markup.italic"),
    ("text.literal", "text.html.markdown markup.raw"),
    ("text.note", "comment"),
    ("text.reference", "text.html.markdown markup.underline.link"),
    ("text.strong", "text.html.markdown markup.bold"),
    ("text.title", "text.html.markdown markup.heading.markdown entity.name.section.markdown"),
    ("text.uri", "text.html.markdown markup.underline.link"),
    ("text.warning", "invalid.deprecated"),
    ("type", "entity.name.type"),
    ("type.builtin", "storage.type"),
    ("type.qualifier", "storage.modifier"),
    ("variable", "variable.other"),
    ("variable.builtin", "variable.language"),
    ("variable.member", "variable.other.member"),
    ("variable.parameter", "variable.parameter"),
];

/// TextMate scope for a capture name, trying shorter prefixes ("function.method.builtin" →
/// "function.method" → "function"); None for captures that only structure the query
/// (names starting with "_", "injection.", "local.", or "none").
pub fn scope_for(capture: &str) -> Option<&'static str> {
    if capture.starts_with('_')
        || capture.starts_with("injection.")
        || capture.starts_with("local.")
        || capture == "none"
    {
        return None;
    }
    let mut name = capture;
    loop {
        if let Some((_, scope)) = TABLE.iter().find(|(n, _)| *n == name) {
            return Some(scope);
        }
        name = &name[..name.rfind('.')?];
    }
}

/// One entry per capture index of `query`: its style under `theme`, None when uncoloured.
pub fn capture_styles(
    query: &tree_sitter::Query,
    theme: &Theme,
    truecolor: bool,
) -> Vec<Option<Style>> {
    let hl = sh::Highlighter::new(theme);
    let default = hl.get_default();
    query
        .capture_names()
        .iter()
        .map(|name| {
            let stack: Vec<Scope> = scope_for(name)?
                .split(' ')
                .map(Scope::new)
                .collect::<Result<_, _>>()
                .ok()?;
            let st = hl.style_for_stack(&stack);
            let plain = st.foreground == default.foreground && st.font_style == FontStyle::empty();
            (!plain).then(|| to_style(st, truecolor))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;
    use std::path::Path;

    #[test]
    fn scope_falls_back_to_shorter_names() {
        assert_eq!(scope_for("function.method.builtin"), scope_for("function.method"));
        assert_eq!(scope_for("keyword.whatever.new"), scope_for("keyword"));
        assert_eq!(scope_for("_hidden"), None);
        assert_eq!(scope_for("injection.content"), None);
    }

    #[test]
    fn material_dark_keyword_is_purple() {
        let theme = crate::highlight::find_theme("material-dark").unwrap(); // make find_theme pub(crate)
        let c = crate::treesitter::langs::compiled(crate::treesitter::langs::for_file(Some(Path::new("x.rs")), "").unwrap());
        let st = capture_styles(&c.highlights, &theme, true);
        let kw = c.highlights.capture_names().iter().position(|n| *n == "keyword").unwrap();
        assert_eq!(st[kw].and_then(|s| s.fg), Some(Color::Rgb(0xc7, 0x92, 0xea)));
    }

    #[test]
    fn every_bundled_capture_name_has_a_scope_or_is_structural() {
        for spec in crate::treesitter::langs::LANGS {
            for name in crate::treesitter::langs::compiled(spec).highlights.capture_names() {
                let structural = name.starts_with('_') || name.starts_with("injection.") || name.starts_with("local.") || *name == "none" || *name == "spell" || *name == "nospell";
                assert!(structural || scope_for(name).is_some(), "{}: @{name}", spec.name);
            }
        }
    }
}
