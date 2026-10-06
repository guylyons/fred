//! org-pcomplete.el: Insert-mode completion of `#+` keywords and their
//! values, `#+begin_src` languages and header arguments, clocktable
//! parameters, tags, TODO keywords, drawer and property names, link
//! abbreviations, entities after `\` and headings after `[[*`. Fred's
//! completion popup shows the candidates (typing, C-n/C-p, M-TAB/C-M-i).
//!
//! Fred extensions: property values after `:NAME: `, and link types
//! after `[[`, next to the link abbreviations.

use super::syntax::{self, Settings};
use super::{Prefix, props};
use crate::editor::{Editor, Mode};

/// org-options-keywords.
const OPTIONS_KEYWORDS: &[&str] = &[
    "ARCHIVE:",
    "AUTHOR:",
    "BIBLIOGRAPHY:",
    "BIND:",
    "CATEGORY:",
    "CITE_EXPORT:",
    "COLUMNS:",
    "CREATOR:",
    "DATE:",
    "DESCRIPTION:",
    "DRAWERS:",
    "EMAIL:",
    "EXCLUDE_TAGS:",
    "FILETAGS:",
    "INCLUDE:",
    "INDEX:",
    "KEYWORDS:",
    "LANGUAGE:",
    "MACRO:",
    "OPTIONS:",
    "PROPERTY:",
    "PRINT_BIBLIOGRAPHY:",
    "PRIORITIES:",
    "SELECT_TAGS:",
    "SEQ_TODO:",
    "SETUPFILE:",
    "STARTUP:",
    "TAGS:",
    "TITLE:",
    "TODO:",
    "TYP_TODO:",
    "SELECT_TAGS:",
    "EXCLUDE_TAGS:",
    "EXPORT_FILE_NAME:",
];

/// org-element-affiliated-keywords.
const AFFILIATED: &[&str] = &[
    "CAPTION", "DATA", "HEADER", "HEADERS", "LABEL", "NAME", "PLOT", "RESNAME", "RESULT",
    "RESULTS", "SOURCE", "SRCNAME", "TBLNAME",
];

/// org-get-export-keywords with the default org-export-backends.
const EXPORT_KEYWORDS: &[&str] = &[
    "SUBTITLE",
    "ATTR_ASCII",
    "ASCII",
    "LATEX_HEADER",
    "CREATOR",
    "INFOJS_OPT",
    "HTML_XML_DECLARATION",
    "HTML_TABLE_ROW_CLOSE_TAG",
    "HTML_TABLE_ROW_OPEN_TAG",
    "HTML_VALIDATION_LINK",
    "HTML_METADATA_TIMESTAMP_FORMAT",
    "HTML_MATHJAX_TEMPLATE",
    "HTML_INFOJS_TEMPLATE",
    "HTML_HOME/UP_FORMAT",
    "HTML_FOOTNOTES_SECTION",
    "HTML_FOOTNOTE_SEPARATOR",
    "HTML_FOOTNOTE_FORMAT",
    "HTML_HEAD_EXTRA",
    "HTML_HEAD",
    "HTML_PREAMBLE",
    "HTML_POSTAMBLE",
    "HTML_EQUATION_REFERENCE_FORMAT",
    "HTML_MATHJAX",
    "HTML_BASE_DIRECTORY",
    "HTML_LINK_UP",
    "HTML_LINK_HOME",
    "KEYWORDS",
    "DESCRIPTION",
    "HTML_CONTENT_CLASS",
    "HTML_CONTAINER",
    "HTML_DOCTYPE",
    "ATTR_HTML",
    "HTML",
    "ICAL-TTL",
    "ICALENDAR_EXCLUDE_TAGS",
    "ATTR_ICALENDAR",
    "ICALENDAR",
    "DATE",
    "LATEX_DESCRIPTIVE_ENV",
    "LATEX_COMPILER",
    "LATEX_ENGRAVED_THEME",
    "LATEX_FOOTNOTE_COMMAND",
    "LATEX_DOC_METADATA",
    "LATEX_CLASS_PRE",
    "LATEX_HEADER_EXTRA",
    "LATEX_CLASS_OPTIONS",
    "LATEX_CLASS",
    "ATTR_LATEX",
    "LATEX",
    "ODT_STYLES_FILE",
    "ATTR_ODT",
    "ODT",
];

/// The keys of org-startup-options.
const STARTUP: &[&str] = &[
    "fold",
    "overview",
    "nofold",
    "showall",
    "show2levels",
    "show3levels",
    "show4levels",
    "show5levels",
    "showeverything",
    "content",
    "indent",
    "noindent",
    "num",
    "nonum",
    "hidestars",
    "showstars",
    "odd",
    "oddeven",
    "align",
    "noalign",
    "shrink",
    "descriptivelinks",
    "literallinks",
    "inlineimages",
    "noinlineimages",
    "linkpreviews",
    "nolinkpreviews",
    "latexpreview",
    "nolatexpreview",
    "customtime",
    "logdone",
    "lognotedone",
    "nologdone",
    "lognoteclock-out",
    "nolognoteclock-out",
    "logrepeat",
    "lognoterepeat",
    "logdrawer",
    "nologdrawer",
    "logstatesreversed",
    "nologstatesreversed",
    "nologrepeat",
    "logreschedule",
    "lognotereschedule",
    "nologreschedule",
    "logredeadline",
    "lognoteredeadline",
    "nologredeadline",
    "logrefile",
    "lognoterefile",
    "nologrefile",
    "fninline",
    "nofninline",
    "fnlocal",
    "fnauto",
    "fnprompt",
    "fnconfirm",
    "fnplain",
    "fnadjust",
    "nofnadjust",
    "fnanon",
    "constcgs",
    "constSI",
    "noptag",
    "hideblocks",
    "nohideblocks",
    "hidedrawers",
    "nohidedrawers",
    "beamer",
    "entitiespretty",
    "entitiesplain",
];

/// The hard-coded #+OPTIONS items.
const OPTIONS: &[&str] = &[
    "H:",
    "\\n:",
    "num:",
    "timestamp:",
    "arch:",
    "author:",
    "c:",
    "creator:",
    "date:",
    "d:",
    "email:",
    "*:",
    "e:",
    "::",
    "f:",
    "inline:",
    "tex:",
    "p:",
    "pri:",
    "':",
    "-:",
    "stat:",
    "^:",
    "toc:",
    "|:",
    "tags:",
    "tasks:",
    "<:",
    "todo:",
];

/// org-html-infojs-opts-table keys.
const INFOJS: &[&str] = &[
    "path", "view", "toc", "ftoc", "tdepth", "sdepth", "mouse", "buttons", "ltoc", "up", "home",
];

/// The languages of org-babel-load-languages' type.
const LANGUAGES: &[&str] = &[
    "awk",
    "C",
    "R",
    "calc",
    "clojure",
    "css",
    "ditaa",
    "dot",
    "emacs-lisp",
    "eshell",
    "forth",
    "fortran",
    "gnuplot",
    "groovy",
    "haskell",
    "java",
    "js",
    "julia",
    "latex",
    "lilypond",
    "lisp",
    "lua",
    "makefile",
    "maxima",
    "ocaml",
    "octave",
    "org",
    "perl",
    "processing",
    "plantuml",
    "python",
    "ruby",
    "sass",
    "scheme",
    "screen",
    "sed",
    "shell",
    "sql",
    "sqlite",
];

/// org-babel-common-header-args-w-values.
const HEADER_ARGS: &[&str] = &[
    "cache",
    "cmdline",
    "colnames",
    "comments",
    "dir",
    "eval",
    "exports",
    "epilogue",
    "file",
    "file-desc",
    "file-ext",
    "file-mode",
    "hlines",
    "mkdirp",
    "no-expand",
    "noeval",
    "noweb",
    "noweb-ref",
    "noweb-sep",
    "noweb-prefix",
    "output-dir",
    "padline",
    "post",
    "prologue",
    "results",
    "rownames",
    "sep",
    "session",
    "shebang",
    "tangle",
    "tangle-mode",
    "var",
    "wrap",
];

const CLOCKTABLE: &[&str] = &[
    ":maxlevel",
    ":scope",
    ":lang",
    ":tstart",
    ":tend",
    ":block",
    ":step",
    ":stepskip0",
    ":fileskip0",
    ":emphasize",
    ":link",
    ":narrow",
    ":indent",
    ":hidefiles",
    ":tcolumns",
    ":level",
    ":compact",
    ":timestamp",
    ":formula",
    ":formatter",
    ":wstart",
    ":mstart",
];

/// org-block-keywords.
fn block_keywords() -> Vec<String> {
    let mut out = vec![];
    for name in [
        "CENTER", "COMMENT", "EXAMPLE", "EXPORT", "QUOTE", "SRC", "VERSE",
    ] {
        out.insert(0, format!("END_{name}"));
        let sp = if matches!(name, "EXPORT" | "SRC") {
            " "
        } else {
            ""
        };
        out.insert(0, format!("BEGIN_{name}{sp}"));
        out.insert(0, format!("ATTR_{name}: "));
    }
    out
}

/// org-pcomplete-case-double.
fn case_double(list: Vec<String>) -> Vec<String> {
    list.into_iter()
        .flat_map(|e| [e.to_lowercase(), e.to_uppercase()])
        .collect()
}

/// pcomplete-uniquify-list (order kept).
fn uniq(list: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for x in list {
        if !out.contains(&x) {
            out.push(x);
        }
    }
    out
}

/// What org-thing-at-point finds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Thing {
    BlockOption(String),
    FileOption(Option<String>),
    Link,
    Tex,
    Tag,
    Todo,
    SearchHead,
    Drawer,
    Prop,
    /// Fred: a property value after `:NAME: `.
    Value(String),
}

/// The property drawer lines of the entry at line `l` contain `l`.
fn in_property_drawer(ed: &Editor, l: usize) -> bool {
    let h = if super::ctx::before_first_heading(ed, l) {
        None
    } else {
        props::entry(ed, l)
    };
    props::drawer(&ed.buf, h).is_some_and(|(s, e)| l > s && l < e)
}

/// org-thing-at-point at byte `b` of line `l`.
pub fn thing_at_point(ed: &Editor, l: usize, b: usize) -> Option<Thing> {
    let line = ed.buf.line(l);
    let b = b.min(line.len());
    let here = &line[..b];
    let rest_blank = line[b..].trim().is_empty();
    let ci = |p: &str| {
        regex::RegexBuilder::new(p)
            .case_insensitive(true)
            .build()
            .unwrap()
    };
    if ci(r"^[ \t]*#\+BEGIN: clocktable[ \t]").is_match(&line) {
        return Some(Thing::BlockOption("clocktable".into()));
    }
    if ci(r"^[ \t]*#\+BEGIN_SRC[ \t]").is_match(&line) {
        return Some(Thing::BlockOption("src".into()));
    }
    if let Some(c) = ci(r"^[ \t]*#\+(\S+?):").captures(&line) {
        return Some(Thing::FileOption(Some(c[1].to_owned())));
    }
    if ci(r"^[ \t]*#\+[a-zA-Z_]*$").is_match(&line) && rest_blank {
        return Some(Thing::FileOption(None));
    }
    let word = here.trim_end_matches(|c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if word.ends_with("[[") {
        return Some(Thing::Link);
    }
    let tex = here
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .trim_end_matches(|c: char| c.is_ascii_alphabetic());
    if tex.ends_with('\\') {
        return Some(Thing::Tex);
    }
    if let Some(c) = crate::org_re!(r"^\*+ (?:.+? )?(:)((?::|[\w@#%]+)+)?[ \t]*$").captures(&line) {
        let g1 = c.get(1).unwrap();
        let in2 = c.get(2).is_some_and(|g| b >= g.start() && b <= g.end());
        if in2 || b == g1.end() {
            return Some(Thing::Tag);
        }
    }
    if crate::org_re!(r"^\*+ +\S*$").is_match(here) && rest_blank {
        return Some(Thing::Todo);
    }
    if let Some(i) = here.rfind('*')
        && here[..i].ends_with("[[")
        && !here[i + 1..].contains('*')
    {
        return Some(Thing::SearchHead);
    }
    if crate::org_re!(r"^[ \t]*:\S*$").is_match(here) && rest_blank {
        return Some(if in_property_drawer(ed, l) {
            Thing::Prop
        } else {
            Thing::Drawer
        });
    }
    if let Some(c) = crate::org_re!(r"^[ \t]*:(\S+):[ \t]+\S*$").captures(here)
        && in_property_drawer(ed, l)
    {
        return Some(Thing::Value(c[1].trim_end_matches('+').to_owned()));
    }
    None
}

/// The whitespace/`[`-separated arguments of `here`: (start, text).
fn arguments(here: &str) -> Vec<(usize, &str)> {
    let mut out = vec![];
    let mut start = None;
    for (i, c) in here.char_indices() {
        let sep = c == ' ' || c == '\t' || c == '[';
        match (sep, start) {
            (true, Some(s)) => {
                out.push((s, &here[s..i]));
                start = None;
            }
            (false, None) => start = Some(i),
            _ => {}
        }
    }
    match start {
        Some(s) => out.push((s, &here[s..])),
        None => out.push((here.len(), "")),
    }
    out
}

/// The completions at the cursor: (start byte of the replaced text, the
/// typed text, candidates matching it).
pub fn candidates(ed: &Editor) -> Option<(usize, String, Vec<String>)> {
    let l = ed.cur.line;
    let line = ed.buf.line(l);
    let b = ed.cur.byte.min(line.len());
    let thing = thing_at_point(ed, l, b)?;
    let here = &line[..b];
    let args = arguments(here);
    let index = args.len() - 1;
    let (arg_start, arg) = args[index];
    let st = super::settings(ed);
    // (start of the completed text, candidates)
    let (start, list): (usize, Vec<String>) = match &thing {
        Thing::FileOption(None) => {
            if index != 0 || !arg.starts_with("#+") {
                return None;
            }
            let mut v: Vec<String> = OPTIONS_KEYWORDS.iter().map(|k| format!("{k} ")).collect();
            v.extend(AFFILIATED.iter().map(|k| format!("{k}: ")));
            v.extend(block_keywords());
            v.extend(EXPORT_KEYWORDS.iter().map(|k| format!("{k}: ")));
            (arg_start + 2, case_double(v))
        }
        Thing::FileOption(Some(name)) => {
            if index == 0 {
                return None;
            }
            (
                arg_start,
                file_option_values(ed, &st, &name.to_lowercase(), index, &args)?,
            )
        }
        Thing::BlockOption(kind) if kind == "src" => {
            if index == 0 {
                return None;
            }
            if index == 1 {
                (
                    arg_start,
                    LANGUAGES.iter().map(|s| (*s).to_owned()).collect(),
                )
            } else {
                let mut v: Vec<String> = HEADER_ARGS.iter().map(|h| format!(":{h}")).collect();
                v.extend(["-n", "-r", "-l"].map(str::to_owned));
                (arg_start, v)
            }
        }
        Thing::BlockOption(_) => {
            if index < 1 {
                return None;
            }
            (
                arg_start,
                CLOCKTABLE.iter().map(|s| (*s).to_owned()).collect(),
            )
        }
        Thing::Link => {
            let mut v: Vec<String> = st.links.iter().map(|(k, _)| format!("{k}:")).collect();
            if let Some(Sexp::List(a)) = super::sexp::option("org-link-abbrev-alist") {
                v.extend(
                    a.iter()
                        .filter_map(|e| Some(format!("{}:", e.car()?.str()?))),
                );
            }
            v.extend(
                super::links::link_types()
                    .into_iter()
                    .map(|t| format!("{t}:")),
            );
            let s = here.rfind("[[").map_or(arg_start, |i| i + 2);
            (s, uniq(v))
        }
        Thing::Tex => {
            let s = here.rfind('\\').map_or(arg_start, |i| i + 1);
            (s, super::entities::names())
        }
        Thing::Todo => (
            arg_start,
            uniq(st.todo_names().into_iter().map(str::to_owned).collect()),
        ),
        Thing::SearchHead => {
            let s = here.rfind('*').map_or(arg_start, |i| i + 1);
            let mut v = vec![];
            for i in 0..ed.line_count() {
                let t = ed.buf.line(i);
                if syntax::level(&t).is_some() {
                    v.push(super::links::normalize(
                        &super::links::heading_text(&t, &st),
                        false,
                        false,
                    ));
                }
            }
            (s, uniq(v))
        }
        Thing::Tag => {
            let s = here.rfind(':').map_or(arg_start, |i| i + 1);
            let mut tags = st.tag_names();
            if tags.is_empty() {
                tags = super::tags::buffer_tags(ed);
            }
            let have = syntax::headline(&line, &st)
                .map(|h| h.tags)
                .unwrap_or_default();
            let v = uniq(tags)
                .into_iter()
                .filter(|t| !have.contains(t))
                .map(|t| format!("{t}:"))
                .collect();
            (s, v)
        }
        Thing::Drawer => {
            let s = here.rfind(':').map_or(arg_start, |i| i + 1);
            let mut names = vec!["PROPERTIES".to_owned()];
            let drawer_re = crate::org_re!(r"^[ \t]*:([\w-]+):[ \t]*$");
            for i in 0..ed.line_count() {
                if let Some(c) = drawer_re.captures(&ed.buf.line(i))
                    && !c[1].eq_ignore_ascii_case("END")
                {
                    names.push(c[1].to_owned());
                }
            }
            let v = uniq(names).into_iter().map(|n| format!("{n}:")).collect();
            (s, case_double(v))
        }
        Thing::Prop => {
            let s = here.rfind(':').map_or(arg_start, |i| i + 1);
            let h = props::entry(ed, l);
            let (ds, de) = props::drawer(&ed.buf, h)?;
            let have: Vec<String> = (ds + 1..de)
                .filter_map(|i| props::parse_property(&ed.buf.line(i)).map(|(k, _)| k))
                .collect();
            let v = uniq(props::buffer_keys(ed, false, true, true))
                .into_iter()
                .filter(|k| !have.contains(k))
                .map(|k| format!("{k}: "))
                .collect();
            (s, case_double(v))
        }
        Thing::Value(key) => (
            arg_start,
            props::values(ed, key)
                .into_iter()
                .filter(|v| !v.is_empty())
                .collect(),
        ),
    };
    let typed = here[start.min(b)..].to_owned();
    let items = uniq(list)
        .into_iter()
        .filter(|c| c.starts_with(&typed))
        .collect();
    Some((start, typed, items))
}

use super::sexp::Sexp;

/// pcomplete/org-mode/file-option/NAME.
fn file_option_values(
    ed: &Editor,
    st: &Settings,
    name: &str,
    index: usize,
    args: &[(usize, &str)],
) -> Option<Vec<String>> {
    let once = index == 1;
    let opt_list = |n: &str, d: &[&str]| -> Vec<String> {
        match super::sexp::option(n) {
            Some(v) => v
                .list()
                .unwrap_or(&[])
                .iter()
                .filter_map(|x| x.str().map(str::to_owned))
                .collect(),
            None => d.iter().map(|s| (*s).to_owned()).collect(),
        }
    };
    let s = |x: String| vec![x];
    Some(match name {
        "author" if once => s(super::options::string(
            "user-full-name",
            &std::env::var("USER").unwrap_or_default(),
        )),
        "date" if once => s(super::time::civil::format(
            &super::time::stamp::time_stamp_format(
                false,
                super::time::stamp::Brackets::Active,
                false,
            ),
            super::time::civil::now(),
        )),
        "email" if once => s(super::options::string("user-mail-address", "")),
        "exclude_tags" if once => s(opt_list("org-export-exclude-tags", &["noexport"]).join(" ")),
        "select_tags" if once => s(opt_list("org-export-select-tags", &["export"]).join(" ")),
        "filetags" if once => s(st.file_tags.join(" ")),
        "language" if once => uniq(vec![
            super::options::string("org-export-default-language", "en"),
            "en".into(),
        ]),
        "priorities" if once => {
            let (h, l, d) = st.priorities;
            s(format!(
                "{} {} {}",
                props::priority_string(h),
                props::priority_string(l),
                props::priority_string(d)
            ))
        }
        "tags" if once => s(tag_alist_to_string(&st.tags)),
        "title" if once => s(ed
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|n| n.to_string_lossy().into_owned())
            .or_else(|| ed.org_buffer_name.clone())
            .unwrap_or_default()),
        "startup" => {
            let mut v: Vec<String> = STARTUP.iter().map(|s| (*s).to_owned()).collect();
            if args.iter().any(|(_, a)| *a == "hidestars") {
                v.retain(|x| x != "showstars");
            }
            v
        }
        "options" => OPTIONS.iter().map(|s| (*s).to_owned()).collect(),
        "infojs_opt" => INFOJS.iter().map(|s| format!("{s}:")).collect(),
        _ => return None,
    })
}

/// org-tag-alist-to-string.
fn tag_alist_to_string(alist: &[super::syntax::TagEntry]) -> String {
    use super::syntax::TagEntry as T;
    alist
        .iter()
        .map(|t| match t {
            T::StartGroup => "{".to_owned(),
            T::EndGroup => "}".to_owned(),
            T::StartGroupTag => "[".to_owned(),
            T::EndGroupTag => "]".to_owned(),
            T::GroupTags => ":".to_owned(),
            T::Newline => "\\n".to_owned(),
            T::Tag(n, Some(k)) => format!("{n}({k})"),
            T::Tag(n, None) => n.clone(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// pcomplete (M-TAB, C-M-i): open the completion popup in Insert mode.
pub fn command(ed: &mut Editor, name: &str, _arg: Prefix) -> Option<Result<(), String>> {
    match name {
        "pcomplete" | "completion-at-point" => {
            if ed.mode != Mode::Insert {
                return Some(Err("pcomplete: completion works in Insert mode".into()));
            }
            crate::vim::insert::refresh_popup(ed, true);
            Some(Ok(()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::org::footnote::tests::at;

    fn items(t: &str) -> (String, Vec<String>) {
        let e = at(t);
        let (_, typed, items) = candidates(&e).unwrap_or_default();
        (typed, items)
    }

    #[test]
    fn keywords_and_values() {
        // Ported from test-org-pcomplete.
        let (typed, v) = items("#+AU<point>");
        assert_eq!(typed, "AU");
        assert_eq!(v, ["AUTHOR: "]);
        assert!(
            items("#+begin_s<point>")
                .1
                .contains(&"begin_src ".to_owned())
        );
        assert!(
            items("#+caption<point>")
                .1
                .contains(&"caption: ".to_owned())
        );
        assert_eq!(items("#+startup: hidesta<point>").1, ["hidestars"]);
        assert!(
            items("#+startup: hidestars <point>")
                .1
                .iter()
                .all(|x| x != "showstars")
        );
        assert_eq!(items("#+options: tod<point>").1, ["todo:"]);
        assert_eq!(items("#+begin_src emacs-l<point>").1, ["emacs-lisp"]);
        assert!(
            items("#+begin_src sh :ta<point>")
                .1
                .contains(&":tangle".to_owned())
        );
        assert_eq!(items("#+BEGIN: clocktable :maxl<point>").1, [":maxlevel"]);
        assert_eq!(items("#+priorities: <point>").1, ["A C B"]);
    }

    #[test]
    fn headings_tags_todo_and_entities() {
        assert_eq!(items("* T<point>").1, ["TODO"]);
        let (_, v) = items("* H :b<point>\n* I :blah:");
        assert_eq!(v, ["blah:"]);
        let (_, v) = items("* H :blah:<point>\n* I :blah:other:");
        assert_eq!(v, ["other:"]);
        assert_eq!(items("\\alp<point>").1, ["alpha"]);
        assert_eq!(items("* First\n* Second\n[[*Sec<point>").1, ["Second"]);
        assert!(items("[[htt<point>").1.contains(&"https:".to_owned()));
    }

    #[test]
    fn drawers_and_properties() {
        assert!(
            items("* H\n:PRO<point>")
                .1
                .contains(&"PROPERTIES:".to_owned())
        );
        let (_, v) =
            items("* H\n:PROPERTIES:\n:pro<point>\n:END:\n* I\n:PROPERTIES:\n:prop: 1\n:END:");
        assert!(v.contains(&"prop: ".to_owned()), "{v:?}");
        let (_, v) = items(
            "* H\n:PROPERTIES:\n:prop: <point>\n:END:\n* I\n:PROPERTIES:\n:prop: val1\n:END:",
        );
        assert_eq!(v, ["val1"]);
    }

    #[test]
    fn popup_in_insert_mode() {
        let e = crate::org::tests::org("", "i#+TI");
        let p = e.popup.as_ref().expect("a popup");
        assert_eq!(p.items, ["TITLE: "]);
        let e = crate::org::tests::org("", "i#+TI<Tab>x");
        assert_eq!(e.buf.text(), "#+TITLE: x");
        // C-n completes entities after a backslash.
        let e = crate::org::tests::org("", "i\\alph<C-n>");
        assert_eq!(e.buf.text(), "\\alpha");
    }
}
