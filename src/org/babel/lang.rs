//! The languages (ob-LANG.el): variable assignment, body expansion,
//! evaluation through external interpreters, and result parsing, each
//! following its upstream file. Which languages run is
//! `org-babel-load-languages` (default: emacs-lisp only).

use super::core::{self, Env, R, err, expand_generic, reassemble};
use super::params::{self, Params, get, get_str, has};
use super::session;
use super::value::{self, Sep, as_string, echo, hline, is_hline, items, list, prin1, princ, str};
use crate::org::options;
use crate::org::sexp::{self, Sexp};
use std::path::Path;

/// org-babel-shell-names.
pub fn shell_names() -> Vec<String> {
    options::strings(
        "org-babel-shell-names",
        &[
            "sh", "bash", "zsh", "fish", "csh", "ash", "dash", "ksh", "mksh", "posh",
        ],
    )
}

/// Languages loaded through org-babel-load-languages, with the ones each
/// library defines.
pub fn loaded() -> Vec<String> {
    let entries: Vec<(String, bool)> = match sexp::option("org-babel-load-languages") {
        Some(v) => items(&v)
            .unwrap_or(&[])
            .iter()
            .filter_map(|e| match e {
                Sexp::Sym(s) | Sexp::Str(s) => Some((s.clone(), true)),
                e => value::car_cdr(e).map(|(k, v)| {
                    let on = match &v {
                        Sexp::List(x) if x.len() == 1 => x[0].truthy(),
                        v => v.truthy(),
                    };
                    (princ(&k), on)
                }),
            })
            .collect(),
        None => vec![("emacs-lisp".into(), true)],
    };
    let mut out = vec![];
    for (lang, on) in entries {
        if !on {
            continue;
        }
        let defined: Vec<String> = match lang.as_str() {
            "emacs-lisp" | "elisp" => vec!["emacs-lisp".into(), "elisp".into()],
            "shell" | "sh" => {
                let mut v = vec!["shell".to_owned()];
                v.extend(shell_names());
                v
            }
            "C" => vec!["C".into(), "C++".into(), "cpp".into(), "D".into()],
            "octave" | "matlab" => vec!["octave".into(), "matlab".into()],
            "clojure" => vec!["clojure".into(), "clojurescript".into()],
            l => vec![l.to_owned()],
        };
        out.extend(defined);
    }
    out
}

/// Every language Fred can execute (each has an org-babel-execute:LANG).
const KNOWN: &[&str] = &[
    "awk",
    "C",
    "C++",
    "cpp",
    "D",
    "calc",
    "clojure",
    "clojurescript",
    "css",
    "ditaa",
    "dot",
    "emacs-lisp",
    "elisp",
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
    "matlab",
    "maxima",
    "ocaml",
    "octave",
    "org",
    "perl",
    "plantuml",
    "processing",
    "python",
    "R",
    "ruby",
    "sass",
    "scheme",
    "screen",
    "sed",
    "shell",
    "sql",
    "sqlite",
    "csharp",
];

/// `(fboundp 'org-babel-execute:LANG)`.
pub fn executable(lang: &str) -> bool {
    (KNOWN.contains(&lang) || shell_names().iter().any(|s| s == lang))
        && loaded().iter().any(|l| l == lang)
}

/// org-babel-result-cond: None discards, Some(true) is the scalar branch.
pub fn result_cond(rp: &[String]) -> Option<bool> {
    let has = |x: &str| rp.iter().any(|r| r == x);
    if has("discard") {
        return None;
    }
    Some(
        ["scalar", "verbatim", "html", "code", "pp", "file"]
            .iter()
            .any(|x| has(x))
            || ((has("output") || has("raw") || has("org")) && !has("table")),
    )
}

fn rp(p: &[(String, Sexp)]) -> Vec<String> {
    params::result_params(p)
}

fn result_type(p: &[(String, Sexp)]) -> &str {
    get(p, ":result-type")
        .and_then(Sexp::sym)
        .unwrap_or("value")
}

fn session_name(p: &[(String, Sexp)]) -> Option<String> {
    match get(p, ":session") {
        None => None,
        Some(Sexp::Nil) => Some(String::new()),
        Some(v) => as_string(v).filter(|s| s != "none"),
    }
}

fn write(path: &Path, text: &str) -> R<()> {
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()).into())
}

fn read_file(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn q(p: &Path) -> String {
    core::shell_quote(&p.display().to_string())
}

/// The scalar result or an imported table (common tail of many languages).
fn scalar_or_table(results: String, p: &[(String, Sexp)], sep: Sep) -> R<Option<Sexp>> {
    Ok(match result_cond(&rp(p)) {
        None => None,
        Some(true) => Some(str(results)),
        Some(false) => Some(value::import_table(&results, sep)?),
    })
}

fn opt(name: &str, default: &str) -> String {
    options::string(name, default)
}

/// A one-line time stamp (format-time-string "%F %T").
pub fn now_stamp() -> String {
    let out = std::process::Command::new("date").arg("+%F %T").output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}

// ---- dispatch ----

/// org-babel-expand-body:LANG, for languages that define it.
pub fn expand(env: &mut Env, lang: &str, body: &str, p: &Params) -> R<Option<String>> {
    Ok(Some(match lang {
        "emacs-lisp" | "elisp" => elisp_expand(body, p),
        "R" => {
            let mut parts = vec![];
            parts.extend(get_str(p, ":prologue"));
            parts.extend(r_assignments(p));
            parts.push(body.to_owned());
            parts.extend(get_str(p, ":epilogue"));
            parts.join("\n")
        }
        "julia" => {
            let mut parts = vec![];
            parts.extend(get_str(p, ":prologue"));
            parts.extend(julia_assignments(p));
            parts.push(body.to_owned());
            parts.extend(get_str(p, ":epilogue"));
            parts.join("\n")
        }
        "dot" | "lilypond" => {
            let mut b = body.to_owned();
            for (n, v) in params::vars(p) {
                b = b.replace(&format!("${n}"), &echo(&v));
            }
            wrap_pro_epi(&b, p)
        }
        "latex" => {
            let mut b = body.to_owned();
            for (n, v) in params::vars(p) {
                b = b.replace(&n, &echo(&v));
            }
            value::trim(&wrap_pro_epi(&b, p)).to_owned()
        }
        "awk" | "calc" => wrap_pro_epi(body, p),
        "sql" => sql_expand(env, body, p, false)?,
        "sqlite" => sql_expand(env, body, p, true)?,
        "org" => {
            let mut b = body.to_owned();
            for (n, v) in params::vars(p) {
                b = b.replace(&format!("${n}"), &princ(&v));
            }
            wrap_pro_epi(&b, p)
        }
        "C" => c_expand(env, body, p, Variant::C)?,
        "C++" | "cpp" => c_expand(env, body, p, Variant::Cpp)?,
        "D" => c_expand(env, body, p, Variant::D)?,
        "fortran" => fortran_expand(env, body, p)?,
        "java" => java_expand(body, p)?,
        "clojure" | "clojurescript" => clojure_expand(body, p),
        "scheme" => {
            let vars = params::vars(p);
            let mut s = String::new();
            if let Some(pro) = get_str(p, ":prologue") {
                s.push_str(&pro);
                s.push('\n');
            }
            if !vars.is_empty() {
                s.push_str(
                    &vars
                        .iter()
                        .map(|(n, v)| format!("(define {n} '{})", prin1(v)))
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
                s.push('\n');
            }
            s.push_str(body);
            if let Some(epi) = get_str(p, ":epilogue") {
                s.push('\n');
                s.push_str(&epi);
            }
            s
        }
        "lisp" => lisp_expand(body, p),
        "gnuplot" => gnuplot_expand(env, body, p)?,
        "maxima" => maxima_expand(body, p),
        "csharp" => csharp_expand(body, p),
        _ => return Ok(None),
    }))
}

fn wrap_pro_epi(body: &str, p: &[(String, Sexp)]) -> String {
    let mut s = String::new();
    if let Some(pro) = get_str(p, ":prologue") {
        s.push_str(&pro);
        s.push('\n');
    }
    s.push_str(body);
    if let Some(epi) = get_str(p, ":epilogue") {
        s.push('\n');
        s.push_str(&epi);
        s.push('\n');
    }
    s
}

/// org-babel-variable-assignments:LANG.
pub fn assignments(env: &mut Env, lang: &str, p: &Params) -> R<Option<Vec<String>>> {
    let vars = params::vars(p);
    Ok(Some(match lang {
        "shell" => sh_assignments(&core::shell_file_name(), p),
        l if shell_names().iter().any(|s| s == l) => sh_assignments(l, p),
        "python" => vars
            .iter()
            .map(|(n, v)| format!("{n}={}", python_var(v)))
            .collect(),
        "ruby" => vars
            .iter()
            .map(|(n, v)| format!("{n}={}", ruby_var(v)))
            .collect(),
        "perl" => vars.iter().map(|(n, v)| perl_var(n, v)).collect(),
        "js" => vars
            .iter()
            .map(|(n, v)| format!("var {n}={};", js_var(v)))
            .collect(),
        "lua" => vars
            .iter()
            .map(|(n, v)| format!("{n}={}", lua_var(v)))
            .collect(),
        "R" => r_assignments(p),
        "julia" => julia_assignments(p),
        "octave" | "matlab" => vars
            .iter()
            .map(|(n, v)| format!("{n}={};", octave_var(v)))
            .collect(),
        "haskell" => vars
            .iter()
            .map(|(n, v)| format!("let {n} = {}", haskell_var(v)))
            .collect(),
        "ocaml" => vars
            .iter()
            .map(|(n, v)| format!("let {n} = {};;", ocaml_var(v)))
            .collect(),
        "java" => java_assignments(p)?,
        "processing" => vars.iter().map(|(n, v)| processing_var(n, v)).collect(),
        "eshell" => vars
            .iter()
            .map(|(n, v)| format!("{n}={}", sh_var_to_sh(v, None, None)))
            .collect(),
        "plantuml" => vars
            .iter()
            .map(|(n, v)| format!("!define {n} {}", princ(v).replace('"', "")))
            .collect(),
        "gnuplot" => gnuplot_vars(env, p)?
            .iter()
            .map(|(n, v)| format!("{n} = \"{v}\""))
            .collect(),
        "csharp" => vars
            .iter()
            .map(|(n, v)| format!("var {n} = {};", prin1(v)))
            .collect(),
        _ => return Ok(None),
    }))
}

/// org-babel-execute:LANG.
pub fn execute(env: &mut Env, lang: &str, body: &str, p: &Params) -> R<Option<Sexp>> {
    match lang {
        "emacs-lisp" | "elisp" => elisp(env, body, p),
        "shell" => shell(env, &core::shell_file_name(), body, p),
        l if shell_names().iter().any(|s| s == l) => shell(env, l, body, p),
        "python" => python(env, body, p),
        "ruby" => ruby(env, body, p),
        "R" => r(env, body, p),
        "sqlite" => sqlite(env, body, p),
        "sql" => sql(env, body, p),
        "dot" => dot(env, body, p),
        "latex" => latex(env, body, p),
        "perl" => perl(env, body, p),
        "js" => js(env, body, p),
        "lua" => lua(env, body, p),
        "julia" => julia(env, body, p),
        "C" => c(env, body, p, Variant::C),
        "C++" | "cpp" => c(env, body, p, Variant::Cpp),
        "D" => c(env, body, p, Variant::D),
        "fortran" => fortran(env, body, p),
        "java" => java(env, body, p),
        "groovy" => groovy(env, body, p),
        "haskell" => haskell(env, body, p),
        "ocaml" => ocaml(env, body, p),
        "clojure" => clojure(env, body, p, false),
        "clojurescript" => clojure(env, body, p, true),
        "scheme" => scheme(env, body, p),
        "lisp" => lisp(env, body, p),
        "forth" => forth(env, body, p),
        "awk" => awk(env, body, p),
        "sed" => sed(env, body, p),
        "calc" => calc(env, body, p),
        "css" | "makefile" => Ok(Some(str(body))),
        "sass" => sass(env, body, p),
        "processing" => processing(env, body, p),
        "screen" => screen(env, body, p),
        "org" => org(env, body, p),
        "eshell" => eshell(env, body, p),
        "octave" => octave(env, body, p, false),
        "matlab" => octave(env, body, p, true),
        "maxima" => maxima(env, body, p),
        "gnuplot" => gnuplot(env, body, p),
        "ditaa" => ditaa(env, body, p),
        "plantuml" => plantuml(env, body, p),
        "lilypond" => lilypond(env, body, p),
        "csharp" => csharp(env, body, p),
        _ => err(format!("No org-babel-execute function for {lang}!")),
    }
}

fn full(env: &mut Env, lang: &str, body: &str, p: &Params) -> R<String> {
    let vars = assignments(env, lang, p)?.unwrap_or_default();
    Ok(expand_generic(body, p, &vars))
}

// ---- emacs-lisp ----

fn elisp_expand(body: &str, p: &[(String, Sexp)]) -> String {
    let vars = params::vars(p);
    if vars.is_empty() {
        return body.to_owned();
    }
    format!(
        "(let ({})\n{}{}{}\n)",
        vars.iter()
            .map(|(n, v)| format!("({n} '{})", prin1(v)))
            .collect::<Vec<_>>()
            .join("\n      "),
        get_str(p, ":prologue")
            .map(|x| format!("{x}\n      "))
            .unwrap_or_default(),
        body,
        get_str(p, ":epilogue")
            .map(|x| format!("\n      {x}\n"))
            .unwrap_or_default()
    )
}

fn elisp(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    if get_str(p, ":session").is_some_and(|s| s != "none") {
        return err("ob-emacs-lisp backend does not support sessions");
    }
    let rp = rp(p);
    let Some(scalar) = result_cond(&rp) else {
        run_elisp(env, body, p, "nil")?;
        return Ok(None);
    };
    let verbatim = rp.iter().any(|r| r == "scalar" || r == "verbatim");
    let output = rp.iter().any(|r| r == "output");
    let pp = rp.iter().any(|r| r == "code" || r == "pp");
    // Values Fred computes without starting Emacs: a variable, or data.
    let vars = params::vars(p);
    let quick = if output || pp {
        None
    } else if let Some((_, v)) = vars.iter().find(|(n, _)| n == body.trim()) {
        Some(v.clone())
    } else if vars.is_empty() && !has(p, ":prologue") && !has(p, ":epilogue") {
        match sexp::read_prefix(body) {
            Ok((form, rest)) if rest.trim().is_empty() && quick_form(&form) => {
                value::eval_lisp_text(body.trim()).ok()
            }
            _ => None,
        }
    } else {
        None
    };
    let result = match quick {
        Some(v) if scalar => str(if verbatim { prin1(&v) } else { princ(&v) }),
        Some(v) => v,
        None => {
            let fmt = if scalar {
                if verbatim {
                    "(format \"%S\" result)"
                } else {
                    "(format \"%s\" result)"
                }
            } else {
                "(prin1-to-string result)"
            };
            let out = run_elisp(env, body, p, fmt)?;
            if scalar {
                str(out)
            } else {
                sexp::read(&out).map_err(|e| format!("Cannot read Emacs value {out:?}: {e}"))?
            }
        }
    };
    if scalar {
        return Ok(Some(result));
    }
    Ok(Some(reassemble(result, p)))
}

fn quick_form(f: &Sexp) -> bool {
    match f {
        Sexp::Str(_) | Sexp::Int(_) | Sexp::Float(_) | Sexp::Nil | Sexp::T | Sexp::Vector(_) => {
            true
        }
        Sexp::Sym(s) => s.starts_with(':'),
        Sexp::List(x) => matches!(x[0].sym(), Some("quote")),
        _ => false,
    }
}

fn run_elisp(env: &mut Env, body: &str, p: &Params, fmt: &str) -> R<String> {
    let rp = rp(p);
    let expanded = elisp_expand(body, p);
    let wrapped = if rp.iter().any(|r| r == "output") {
        format!("(with-output-to-string {expanded}\n)")
    } else {
        format!("(progn {expanded}\n)")
    };
    let wrapped = if rp.iter().any(|r| r == "code" || r == "pp") {
        format!("(pp-to-string {wrapped})")
    } else {
        wrapped
    };
    let lexical = match get(p, ":lexical") {
        Some(Sexp::Str(s)) | Some(Sexp::Sym(s)) => {
            if s == "yes" || s == "t" {
                "t".to_owned()
            } else {
                "nil".to_owned()
            }
        }
        Some(Sexp::T) => "t".into(),
        Some(v) if value::listp(v) && !v.is_nil() => format!("'{}", prin1(v)),
        _ => "nil".into(),
    };
    let program = format!(
        "(let* ((print-length nil) (print-level nil) (default-directory {}) (result (eval (car (read-from-string {})) {lexical}))) (princ {fmt}))",
        prin1(&str(format!("{}/", env.dir.display()))),
        prin1(&str(wrapped))
    );
    let out = std::process::Command::new(value::emacs_program())
        .args(["--batch", "-Q", "--eval", &program])
        .current_dir(&env.dir)
        .output()
        .map_err(|e| {
            format!(
                "ob-emacs-lisp needs Emacs ({}): {e}",
                value::emacs_program()
            )
        })?;
    if !out.status.success() {
        let e = String::from_utf8_lossy(&out.stderr);
        return err(e
            .lines()
            .rfind(|l| !l.trim().is_empty())
            .unwrap_or("Emacs Lisp evaluation failed")
            .to_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Run an upstream babel function in `emacs --batch` (calc).
fn emacs_babel(env: &mut Env, feature: &str, func: &str, body: &str, p: &Params) -> R<Sexp> {
    let alist = list(
        p.iter()
            .map(|(k, v)| value::cons(Sexp::Sym(k.clone()), v.clone()))
            .collect(),
    );
    let program = format!(
        "(progn (require '{feature}) (let ((print-length nil) (print-level nil)) (princ (prin1-to-string ({func} {} '{})))))",
        prin1(&str(body)),
        prin1(&alist)
    );
    let out = std::process::Command::new(value::emacs_program())
        .args(["--batch", "-Q", "--eval", &program])
        .current_dir(&env.dir)
        .output()
        .map_err(|e| format!("{func} needs Emacs ({}): {e}", value::emacs_program()))?;
    if !out.status.success() {
        let e = String::from_utf8_lossy(&out.stderr);
        return err(e
            .lines()
            .rfind(|l| !l.trim().is_empty())
            .unwrap_or("Emacs evaluation failed")
            .to_owned());
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    sexp::read(&text).map_err(|e| format!("Cannot read Emacs value {text:?}: {e}").into())
}

// ---- shell ----

fn sh_var_to_string(v: &Sexp, sep: Option<&str>, hl: Option<&str>) -> String {
    match items(v) {
        Some(x) if x.first().is_some_and(|f| value::listp(f) || is_hline(f)) => {
            value::orgtbl_to_generic(v, sep.unwrap_or("\t"), &echo, hl)
        }
        Some(x) if !v.is_nil() => x.iter().map(echo).collect::<Vec<_>>().join("\n"),
        _ => echo(v),
    }
}

fn sh_var_to_sh(v: &Sexp, sep: Option<&str>, hl: Option<&str>) -> String {
    format!(
        "'{}'",
        sh_var_to_string(v, sep, hl).replace('\'', "'\"'\"'")
    )
}

fn sh_assignments(shell: &str, p: &[(String, Sexp)]) -> Vec<String> {
    let sep = get_str(p, ":separator");
    let hl = (get_str(p, ":hlines").as_deref() == Some("yes"))
        .then(|| get_str(p, ":hline-string").unwrap_or_else(|| "hline".into()));
    let (sep, hl) = (sep.as_deref(), hl.as_deref());
    params::vars(p)
        .iter()
        .map(|(n, v)| {
            if shell.ends_with("bash") {
                match items(v) {
                    Some(x) if x.first().and_then(items).is_some_and(|r| r.len() >= 2) => {
                        let rows: Vec<String> = x
                            .iter()
                            .filter_map(value::car_cdr)
                            .map(|(k, rest)| {
                                format!(
                                    "{n}[{}]={}",
                                    sh_var_to_sh(&k, sep, hl),
                                    sh_var_to_sh(&rest, sep, hl)
                                )
                            })
                            .collect();
                        format!("unset {n}\ndeclare -A {n}\n{}", rows.join("\n"))
                    }
                    Some(x) if !x.is_empty() => {
                        format!(
                            "unset {n}\ndeclare -a {n}=( {} )",
                            x.iter()
                                .map(|e| sh_var_to_sh(e, sep, hl))
                                .collect::<Vec<_>>()
                                .join(" ")
                        )
                    }
                    _ => format!("{n}={}", sh_var_to_sh(v, sep, hl)),
                }
            } else if shell.ends_with("fish") {
                format!("set {n} {}", sh_var_to_sh(v, sep, hl))
            } else {
                format!("{n}={}", sh_var_to_sh(v, sep, hl))
            }
        })
        .collect()
}

fn shell(env: &mut Env, shell: &str, body: &str, p: &Params) -> R<Option<Sexp>> {
    let rp = rp(p);
    let session = session_name(p);
    let stdin = match get_str(p, ":stdin") {
        Some(s) => Some(sh_var_to_string(&env.resolve(&s)?, None, None)),
        None => None,
    };
    let exit_status = (rp == ["replace"]
        && !options::bool("org-babel-shell-results-defaults-to-output", true))
        || rp.iter().any(|r| r == "value");
    let cmdline = get_str(p, ":cmdline");
    let mut fb = expand_generic(body, p, &sh_assignments(shell, p));
    if exit_status {
        fb.push_str("\necho $?");
    }
    let shebang = get_str(p, ":shebang").filter(|s| !s.is_empty());
    let padline = get_str(p, ":padline").as_deref() != Some("no");
    let mut results = if stdin.is_some() || cmdline.is_some() {
        let script = env.temp_file("sh-script-", "");
        let mut text = shebang
            .clone()
            .unwrap_or_else(|| format!("#!/usr/bin/env {shell}"));
        text.push('\n');
        if padline {
            text.push('\n');
        }
        text.push_str(&fb);
        write(&script, &text)?;
        chmod(&script, 0o755);
        let cmd = format!(
            "{} -c {}",
            core::shell_quote(shell),
            core::shell_quote(&format!(
                "{} {}",
                script.display(),
                cmdline.unwrap_or_default()
            ))
        );
        let (out, errs, _) = core::run_shell(&cmd, stdin.as_deref().unwrap_or(""), &env.dir)?;
        out + &errs
    } else if let Some(name) = session {
        let name = if name.is_empty() {
            format!("*{shell}*")
        } else {
            name
        };
        session::eval(
            &session::Kind::Shell(shell.to_owned()),
            &name,
            &env.dir,
            value::trim(&fb),
        )?
    } else if let Some(sb) = shebang {
        let script = env.temp_file("sh-script-", "");
        write(
            &script,
            &format!("{sb}\n{}{fb}", if padline { "\n" } else { "" }),
        )?;
        chmod(&script, 0o755);
        env.eval(&q(&script), "")?
    } else {
        env.eval(&core::shell_quote(shell), value::trim(&fb))?
    };
    if exit_status {
        results = results
            .split('\n')
            .filter(|l| !l.is_empty())
            .next_back()
            .unwrap_or("")
            .to_owned();
    }
    let r = scalar_or_table(results, p, Sep::Auto)?;
    Ok(r.map(|r| reassemble(r, p)))
}

fn chmod(p: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode));
}

fn eshell(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let fb = full(env, "eshell", body, p)?;
    let out = match session_name(p) {
        Some(name) => session::eval(
            &session::Kind::Shell(core::shell_file_name()),
            &name,
            &env.dir,
            &fb,
        )?,
        None => env.eval(&core::shell_quote(&core::shell_file_name()), &fb)?,
    };
    Ok(Some(str(out)))
}

// ---- python ----

pub const PYTHON_UTILS: &str = r#"import ast
import pprint

def session_results_output(src_file_name, context):
    with open(src_file_name) as f:
        exec(compile(f.read(), f.name, 'exec'), context)

def session_results_value(src_file_name, result_file_name, result_params, context):
    with open(src_file_name) as f:
        src_ast = ast.parse(f.read())
    final_line = src_ast.body[-1] if src_ast.body else None
    if isinstance(final_line, ast.Expr):
        src_ast.body = src_ast.body[:-1]
        exec(compile(src_ast, '<string>', 'exec'), context)
        final_line = eval(compile(ast.Expression(final_line.value), '<string>', 'eval'), context)
    else:
        exec(compile(src_ast, '<string>', 'exec'), context)
        final_line = None
    format_results_value(final_line, result_file_name, result_params)

def clear_graphics():
    import matplotlib.pyplot
    matplotlib.pyplot.gcf().clear()

def save_graphics(file_name):
    import matplotlib.pyplot
    matplotlib.pyplot.savefig(file_name)

def format_results_value(result, result_file_name, result_params):
    with open(result_file_name, 'w') as result_file_obj:
        if 'graphics' in result_params:
            result.savefig(result_file_name)
        elif 'pp' in result_params:
            result_file_obj.write(pprint.pformat(result))
        elif 'list' in result_params and isinstance(result, dict):
            result_file_obj.write(str(['{} :: {}'.format(k, v) for k, v in result.items()]))
        else:
            if not set(result_params).intersection(['scalar', 'verbatim', 'raw']):
                def dict2table(res):
                    if isinstance(res, dict):
                        return [(k, dict2table(v)) for k, v in res.items()]
                    elif isinstance(res, list) or isinstance(res, tuple):
                        return [dict2table(x) for x in res]
                    else:
                        return res
                if 'table' in result_params:
                    result = dict2table(result)
                try:
                    import pandas
                except ImportError:
                    pass
                else:
                    if isinstance(result, pandas.DataFrame) and 'table' in result_params:
                        result = [[result.index.name or ''] + list(result.columns)] + [None] + [[i] + list(row) for i, row in result.iterrows()]
                    elif isinstance(result, pandas.Series) and 'table' in result_params:
                        result = list(result.items())
                try:
                    import numpy
                except ImportError:
                    pass
                else:
                    if isinstance(result, numpy.ndarray):
                        if 'table' in result_params:
                            result = result.tolist()
                        else:
                            result = repr(result)
            result_file_obj.write(str(result))
"#;

/// org-babel-python--load-utils.
pub fn python_load_utils() -> String {
    let dir = core::temp_dir();
    let _ = std::fs::write(dir.join("ob_python_utils.py"), PYTHON_UTILS);
    format!(
        "def __OB_PYTHON_setup_path(dir):\n    import sys\n    if not dir in sys.path:\n        sys.path.append(dir)\n__OB_PYTHON_setup_path('{}')\nimport ob_python_utils as __OB_PYTHON",
        dir.display()
    )
}

fn python_var(v: &Sexp) -> String {
    if let Some(x) = items(v).filter(|_| value::listp(v)) {
        return format!(
            "[{}]",
            x.iter().map(python_var).collect::<Vec<_>>().join(", ")
        );
    }
    if is_hline(v) {
        return opt("org-babel-python-hline-to", "None");
    }
    match v {
        Sexp::Str(s) if s.contains(['\n', '\r']) => format!("\"\"{}\"\"", prin1(v)),
        _ => prin1(v),
    }
}

fn python_table_or_string(results: &str) -> Sexp {
    let res = if results.starts_with('{') {
        str(results)
    } else {
        value::script_escape(results, false)
    };
    match items(&res) {
        Some(x) if value::listp(&res) && !res.is_nil() => {
            let none_to = opt("org-babel-python-None-to", "hline");
            list(
                x.iter()
                    .map(|e| {
                        if matches!(e, Sexp::Sym(s) if s == "None") {
                            Sexp::Sym(none_to.clone())
                        } else {
                            e.clone()
                        }
                    })
                    .collect(),
            )
        }
        _ => res,
    }
}

/// org-babel-python--shift-right: indent lines not inside strings.
fn python_shift_right(body: &str, count: usize) -> String {
    let pad = " ".repeat(count);
    let mut in_triple: Option<&str> = None;
    let mut out = vec![];
    for line in body.split('\n') {
        let inside = in_triple.is_some();
        out.push(if inside || line.is_empty() {
            line.to_owned()
        } else {
            format!("{pad}{line}")
        });
        // Track triple-quoted strings (single-line strings close on their line).
        let mut i = 0;
        let b = line.as_bytes();
        while i < b.len() {
            if let Some(t) = in_triple {
                if line[i..].starts_with(t) {
                    in_triple = None;
                    i += 3;
                    continue;
                }
                i += 1;
                continue;
            }
            match b[i] {
                b'#' => break,
                b'"' | b'\'' => {
                    let t = if b[i] == b'"' { "\"\"\"" } else { "'''" };
                    if line[i..].starts_with(t) {
                        in_triple = Some(t);
                        i += 3;
                        continue;
                    }
                    let qch = b[i];
                    i += 1;
                    while i < b.len() && b[i] != qch {
                        if b[i] == b'\\' {
                            i += 1;
                        }
                        i += 1;
                    }
                    i += 1;
                }
                _ => i += 1,
            }
        }
    }
    out.join("\n")
}

fn python_command(p: &[(String, Sexp)], session: bool) -> String {
    if let Some(c) = get_str(p, ":python") {
        return c;
    }
    let main = opt("org-babel-python-command", "auto");
    if main != "auto" {
        return main;
    }
    if session {
        let s = opt("org-babel-python-command-session", "auto");
        if s != "auto" {
            return s;
        }
        return opt("python-shell-interpreter", "python3");
    }
    opt("org-babel-python-command-nonsession", "python")
}

const PY_GRAPHICS: &str = "__OB_PYTHON.clear_graphics()\n%s\n__OB_PYTHON.save_graphics('%f')";

fn python(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let session = session_name(p);
    let rp = rp(p);
    let graphics = if rp.iter().any(|r| r == "graphics") {
        core::graphical_output_file(p)?
    } else {
        None
    };
    let graphics = graphics.map(|g| env.dir.join(g).display().to_string());
    let rtype = result_type(p).to_owned();
    let return_val = if rtype == "value" {
        get_str(p, ":return")
    } else {
        None
    };
    let preamble = get_str(p, ":preamble");
    let mut fb = full(env, "python", body, p)?;
    if let Some(r) = return_val {
        fb.push_str(&if session.is_some() {
            format!("\n{r}")
        } else {
            format!("\nreturn {r}")
        });
    }
    let utils = python_load_utils();
    let rp_py = python_var(&list(rp.iter().map(|s| str(s.clone())).collect()));
    let raw = if let Some(name) = session {
        let name = if name.is_empty() {
            "*Python*".to_owned()
        } else {
            with_earmuffs(&name)
        };
        let kind = session::Kind::Python(python_command(p, true));
        session::eval(&kind, &name, &env.dir, &utils)?;
        let src = env.temp_file("python-", "");
        let code = match (&graphics, rtype.as_str()) {
            (Some(g), "output") => PY_GRAPHICS.replace("%s", &fb).replace("%f", g),
            _ => fb.clone(),
        };
        write(&src, &code)?;
        if rtype == "output" {
            let out = session::eval(
                &kind,
                &name,
                &env.dir,
                &format!(
                    "__OB_PYTHON.session_results_output('{}', globals())",
                    src.display()
                ),
            )?;
            value::chomp(out.trim_end(), "\n").to_owned()
        } else {
            let res = graphics
                .clone()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| env.temp_file("python-", ""));
            session::eval(
                &kind,
                &name,
                &env.dir,
                &format!(
                    "__OB_PYTHON.session_results_value('{}', '{}', {rp_py}, globals())",
                    src.display(),
                    res.display()
                ),
            )?;
            read_file(&res)
        }
    } else {
        let cmd = python_command(p, false);
        let pre = preamble.map(|x| x + "\n").unwrap_or_default();
        if rtype == "output" {
            let code = match &graphics {
                Some(g) => format!(
                    "{utils}\n{}",
                    PY_GRAPHICS.replace("%s", &fb).replace("%f", g)
                ),
                None => fb.clone(),
            };
            env.eval(&cmd, &format!("{pre}{code}"))?
        } else {
            let res = graphics
                .clone()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| env.temp_file("python-", ""));
            let code = format!(
                "{pre}{utils}\ndef main():\n{}\n\n__OB_PYTHON.format_results_value(main(), '{}', {rp_py})",
                python_shift_right(&fb, 4),
                res.display()
            );
            env.eval(&cmd, &code)?;
            read_file(&res)
        }
    };
    let r = match result_cond(&rp) {
        None => None,
        Some(true) => Some(str(raw)),
        Some(false) => Some(python_table_or_string(&raw)),
    };
    Ok(r.map(|r| reassemble(r, p)))
}

fn with_earmuffs(s: &str) -> String {
    if s.starts_with('*') && s.ends_with('*') && s.len() > 1 {
        s.to_owned()
    } else {
        format!("*{s}*")
    }
}

// ---- ruby ----

fn ruby_var(v: &Sexp) -> String {
    if let Some(x) = items(v).filter(|_| value::listp(v)) {
        return format!(
            "[{}]",
            x.iter().map(ruby_var).collect::<Vec<_>>().join(", \n")
        );
    }
    if is_hline(v) {
        opt("org-babel-ruby-hline-to", "nil")
    } else {
        prin1(v)
    }
}

fn ruby_table_or_string(results: &str) -> Sexp {
    let res = value::script_escape(results, false);
    match items(&res) {
        Some(x) if !res.is_nil() => {
            let nil_to = opt("org-babel-ruby-nil-to", "hline");
            list(
                x.iter()
                    .map(|e| {
                        if e.is_nil() {
                            Sexp::Sym(nil_to.clone())
                        } else {
                            e.clone()
                        }
                    })
                    .collect(),
            )
        }
        _ => res,
    }
}

const RUBY_WRAPPER: &str = "\nresults = (lambda do\n%s\nend).call\nFile.open('%f', 'w'){ |f| f.write((results.class == String) ? results : results.inspect) }\n";
const RUBY_PP_WRAPPER: &str = "\nrequire 'pp'\nresults = (lambda do\n%s\nend).call\nFile.open('%f', 'w') do |f|\n  $stdout = f\n  pp results\nend\n";

fn ruby(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let rp = rp(p);
    let cmd = get_str(p, ":ruby").unwrap_or_else(|| opt("org-babel-ruby-command", "ruby"));
    let fb = full(env, "ruby", body, p)?;
    let result = if let Some(name) = session_name(p) {
        let name = if name.is_empty() {
            "ruby".to_owned()
        } else {
            name
        };
        let kind = session::Kind::Ruby(cmd.clone());
        if result_type(p) == "output" {
            let out = session::eval(&kind, &name, &env.dir, value::chomp(&fb, value::WS))?;
            out.split(['\r', '\n'])
                .map(value::trim)
                .collect::<Vec<_>>()
                .join("\n")
                .trim_end()
                .to_owned()
        } else {
            let tmp = env.temp_file("ruby-", "");
            session::eval(&kind, &name, &env.dir, value::chomp(&fb, value::WS))?;
            let w = if rp.iter().any(|r| r == "code" || r == "pp") {
                format!(
                    "results=$ob_last\nrequire 'pp'\norig_out = $stdout\nFile.open('{}','w'){{|f| $stdout = f; pp(results); $stdout = orig_out}}",
                    tmp.display()
                )
            } else {
                format!(
                    "File.open('{}','w'){{|f| f.write(($ob_last.class == String) ? $ob_last : $ob_last.inspect)}}",
                    tmp.display()
                )
            };
            session::eval(&kind, &name, &env.dir, &w)?;
            read_file(&tmp)
        }
    } else if result_type(p) == "output" {
        env.eval(&cmd, &fb)?
    } else {
        let tmp = env.temp_file("ruby-", "");
        let w = if rp.iter().any(|r| r == "pp") {
            RUBY_PP_WRAPPER
        } else {
            RUBY_WRAPPER
        };
        env.eval(
            &cmd,
            &w.replace("%s", &fb)
                .replace("%f", &tmp.display().to_string()),
        )?;
        read_file(&tmp)
    };
    let r = match result_cond(&rp) {
        None => None,
        Some(true) => Some(str(result)),
        Some(false) => Some(ruby_table_or_string(&result)),
    };
    Ok(r.map(|r| reassemble(r, p)))
}

// ---- perl ----

fn perl_var(n: &str, v: &Sexp) -> String {
    fn inner(v: &Sexp, lvl: usize) -> String {
        let prefix = " ".repeat(2 * lvl);
        let mut s = prefix.clone();
        if let Some(x) = items(v).filter(|_| value::listp(v)) {
            s.push_str("[\n");
            for e in x {
                s.push_str(&inner(e, lvl + 1));
            }
            s.push_str(&prefix);
            s.push(']');
        } else {
            s.push_str(&format!("q({})", princ(v)));
        }
        if lvl > 0 {
            s.push_str(",\n");
        }
        s
    }
    format!(
        "my ${n}={}{};\n",
        if value::listp(v) { "\n" } else { "" },
        inner(v, 0)
    )
}

const PERL_WRAPPER: &str = "{\n    my $babel_sub = sub {\n        %s\n    };\n    open my $BOH, qq(>%f) or die qq(Perl: Could not open output file.$/);\n    my $rv = &$babel_sub();\n    my $rt = ref $rv;\n    select $BOH;\n    if (qq(ARRAY) eq $rt) {\n        local $\\=$/;\n        local $,=qq(\\t);\n\tforeach my $rv ( @$rv ) {\n\t    my $rt = ref $rv;\n\t    if (qq(ARRAY) eq $rt) {\n\t\tprint @$rv;\n\t    } else {\n\t\tprint $rv;\n\t    }\n\t}\n    } else {\n\tprint $rv;\n    }\n}";

fn perl(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    if session_name(p).is_some() {
        return err("Sessions are not supported for Perl");
    }
    let fb = full(env, "perl", body, p)?;
    let tmp = env.temp_file("perl-", "");
    let cmd = opt("org-babel-perl-command", "perl");
    let pre = opt("org-babel-perl-preface", "");
    let pre = if pre == "nil" { String::new() } else { pre };
    if result_type(p) == "output" {
        let out = env.eval(&cmd, &format!("{pre}{fb}"))?;
        write(&tmp, &out)?;
    } else {
        env.eval(
            &cmd,
            &PERL_WRAPPER
                .replace("%s", &format!("{pre}{fb}"))
                .replace("%f", &tmp.display().to_string()),
        )?;
    }
    let text = read_file(&tmp);
    let r = match result_cond(&rp(p)) {
        None => None,
        Some(true) => Some(str(text)),
        Some(false) => Some(value::import_table(&text, Sep::Tab)?),
    };
    Ok(r.map(|r| reassemble(r, p)))
}

// ---- js ----

fn js_var(v: &Sexp) -> String {
    if let Some(x) = items(v).filter(|_| value::listp(v)) {
        return format!("[{}]", x.iter().map(js_var).collect::<Vec<_>>().join(", "));
    }
    prin1(v).replace('\n', "\\n")
}

fn js_read(results: &str) -> Sexp {
    if results.starts_with('[') && results.ends_with(']') {
        let t = results.replace('\'', "\"");
        let t = regex::Regex::new(r",\s")
            .unwrap()
            .replace_all(&t, " ")
            .replace(']', ")")
            .replace('[', "(");
        return value::read(&format!("'{t}"), false).unwrap_or_else(|_| str(results));
    }
    value::read(results, false).unwrap_or_else(|_| str(results))
}

fn js(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let cmd = get_str(p, ":cmd").unwrap_or_else(|| opt("org-babel-js-cmd", "node"));
    let fb = full(env, "js", body, p)?;
    let value = result_type(p) == "value";
    let result = match session_name(p) {
        None => {
            let script = env.temp_file("js-script-", "");
            let code = if value {
                format!(
                    "require('process').stdout.write(require('util').inspect(function(){{{fb}\n}}()));"
                )
            } else {
                fb
            };
            write(&script, &code)?;
            env.eval(&format!("{cmd} {}", q(&script)), "")?
        }
        Some(name) => {
            if cmd != "node" {
                return err("Sessions are only supported with node");
            }
            let name = if name.is_empty() {
                "*Javascript REPL*".to_owned()
            } else {
                name
            };
            let kind = session::Kind::Node(cmd);
            let out = session::eval(&kind, &name, &env.dir, &fb)?;
            if value {
                session::eval(
                    &kind,
                    &name,
                    &env.dir,
                    "process.stdout.write(require('util').inspect(__ob_last))",
                )?
            } else {
                out
            }
        }
    };
    Ok(match result_cond(&rp(p)) {
        None => None,
        Some(true) => Some(str(result)),
        Some(false) => Some(js_read(&result)),
    })
}

// ---- lua ----

fn lua_var(v: &Sexp) -> String {
    if let Some(x) = items(v).filter(|_| value::listp(v)) {
        if x.len() == 1 && !value::listp(&x[0]) {
            return lua_var(&x[0]);
        }
        if x.len() == 2 && !value::listp(&x[0]) {
            return format!("{}={}", princ(&x[0]), lua_var(&list(x[1..].to_vec())));
        }
        return format!(
            "{{{}}}",
            x.iter().map(lua_var).collect::<Vec<_>>().join(", ")
        );
    }
    if is_hline(v) {
        return opt("org-babel-lua-hline-to", "None");
    }
    match v {
        Sexp::Str(s) if s.contains(['\n', '\r']) => format!("[=[{s}]=]"),
        _ => prin1(v),
    }
}

const LUA_WRAPPER: &str = "\nfunction main()\n%s\nend\nfunction dump(it, indent)\n   if indent == nil then\n      indent = ''\n   end\n   if type(it) == 'table' and %pp then\n      local result = ''\n      if #indent ~= 0 then\n         result = result .. '\\n'\n      end\n      local keys = {}\n      for key in pairs(it) do\n        table.insert(keys, key)\n      end\n      table.sort(keys)\n      for index, key in pairs(keys) do\n         local value = it[key]\n         result = result\n            .. indent\n            .. dump(key)\n            .. ' = '\n            .. dump(value, indent .. '  ')\n         if index ~= #keys then\n            result = result .. '\\n'\n         end\n      end\n      return result\n   else\n      return string.gsub(tostring(it), '\"', '\\\"')\n   end\nend\nfunction combine(...)\n  local result = {}\n  for index = 1, select('#', ...) do\n    result[index] = dump(select(index, ...))\n  end\n  if #result == 1 then\n    local value = result[1]\n    if string.find(value, '[%(%[{]') == 1 then\n      return '\"' .. value .. '\"'\n    else\n      return value\n    end\n  end\n  return '\"' .. table.concat(result, '%sep') .. '\"'\nend\noutput = io.open('%f', 'w')\noutput:write(combine(main()))\noutput:close()";

fn lua(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    if session_name(p).is_some() {
        return err("Sessions not supported for Lua");
    }
    let rp = rp(p);
    let rv = if result_type(p) == "value" {
        get_str(p, ":return")
            .map(|r| format!("\nreturn {r}"))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let fb = full(env, "lua", &format!("{body}{rv}"), p)?;
    let cmd = opt("org-babel-lua-command", "lua");
    let pre = get_str(p, ":preamble")
        .map(|x| x + "\n")
        .unwrap_or_default();
    let raw = if result_type(p) == "output" {
        env.eval(&cmd, &format!("{pre}{fb}"))?
    } else {
        let tmp = env.temp_file("lua-", "");
        let body = value::remove_indentation(value::trim(&fb))
            .split(['\r', '\n'])
            .map(|l| format!("\t{l}"))
            .collect::<Vec<_>>()
            .join("\n");
        let code = LUA_WRAPPER
            .replace(
                "%pp",
                if rp.iter().any(|r| r == "pp") {
                    "true"
                } else {
                    "false"
                },
            )
            .replace(
                "%sep",
                &opt("org-babel-lua-multiple-values-separator", ", "),
            )
            .replace("%f", &tmp.display().to_string())
            .replace("%s", &body);
        env.eval(&cmd, &format!("{pre}{code}"))?;
        read_file(&tmp)
    };
    let r = match result_cond(&rp) {
        None => None,
        Some(true) => Some(str(raw)),
        Some(false) => {
            let res = value::script_escape(value::trim(&raw), false);
            Some(match items(&res) {
                Some(x) if !res.is_nil() => list(
                    x.iter()
                        .map(|e| {
                            if matches!(e, Sexp::Sym(s) if s == "None") {
                                Sexp::Sym(opt("org-babel-lua-None-to", "hline"))
                            } else {
                                e.clone()
                            }
                        })
                        .collect(),
                ),
                _ => res,
            })
        }
    };
    Ok(r.map(|r| reassemble(r, p)))
}

// ---- R ----

fn r_quote(v: &Sexp) -> String {
    match v {
        Sexp::Str(s) => format!("\"{}\"", s.replace('"', "\"\"")),
        v => prin1(v),
    }
}

fn r_assign(name: &str, v: &Sexp, colnames: bool, rownames: bool) -> String {
    if value::listp(v) {
        let x = items(v).unwrap_or(&[]);
        let lengths: Vec<usize> = x
            .iter()
            .filter_map(|e| items(e).filter(|_| value::listp(e)).map(<[Sexp]>::len))
            .collect();
        let max = lengths.iter().copied().max().unwrap_or(0);
        let min = lengths.iter().copied().min().unwrap_or(0);
        let table = if x.first().is_some_and(|f| value::listp(f)) {
            v.clone()
        } else {
            list(x.iter().map(|e| list(vec![e.clone()])).collect())
        };
        let tsv = value::orgtbl_to_generic(&table, "\t", &r_quote, None);
        let header = if items(&table).and_then(|t| t.get(1)).is_some_and(is_hline) || colnames {
            "TRUE"
        } else {
            "FALSE"
        };
        let row_names = if rownames { "1" } else { "NULL" };
        let tsv = prin1(&str(tsv));
        if max == min {
            format!(
                "{name} <- local({{\n     con <- textConnection(\n       {tsv}\n     )\n     res <- utils::read.table(\n       con,\n       header    = {header},\n       row.names = {row_names},\n       sep       = \"\\t\",\n       as.is     = TRUE\n     )\n     close(con)\n     res\n   }})"
            )
        } else {
            format!(
                "{name} <- local({{\n     con <- textConnection(\n       {tsv}\n     )\n     res <- utils::read.table(\n       con,\n       header    = {header},\n       row.names = {row_names},\n       sep       = \"\\t\",\n       as.is     = TRUE,\n       fill      = TRUE,\n       col.names = paste(\"V\", seq_len({max}), sep =\"\")\n     )\n     close(con)\n     res\n   }})"
            )
        }
    } else {
        match v {
            Sexp::Int(i) => format!("{name} <- {i}L"),
            Sexp::Float(_) => format!("{name} <- {}", prin1(v)),
            Sexp::Str(_) => format!("{name} <- {}", prin1(v)),
            v => format!("{name} <- {}", prin1(&str(prin1(v)))),
        }
    }
}

fn r_assignments(p: &[(String, Sexp)]) -> Vec<String> {
    let vars = params::vars(p);
    let cn = items(get(p, ":colname-names").unwrap_or(&Sexp::Nil))
        .unwrap_or(&[])
        .to_vec();
    let rn = items(get(p, ":rowname-names").unwrap_or(&Sexp::Nil))
        .unwrap_or(&[])
        .to_vec();
    let colnames = get_str(p, ":colnames").as_deref() == Some("yes");
    let rownames = get_str(p, ":rownames").as_deref() == Some("yes");
    vars.iter()
        .enumerate()
        .map(|(i, (n, v))| {
            let c = cn.get(i).and_then(value::car_cdr).map(|(_, x)| x);
            let r = rn.get(i).and_then(value::car_cdr).map(|(_, x)| x);
            r_assign(
                n,
                &core::reassemble_table(v.clone(), c, r),
                colnames,
                rownames,
            )
        })
        .collect()
}

const R_WRITE: &str = "{\n    function(object,transfer.file) {\n        object\n        invisible(\n            if (\n                inherits(\n                    try(\n                        {\n                            tfile<-tempfile()\n                            write.table(object, file=tfile, sep=\"\\t\",\n                                        na=\"\",row.names=%r,col.names=%c,\n                                        quote=FALSE)\n                            file.rename(tfile,transfer.file)\n                        },\n                        silent=TRUE),\n                    \"try-error\"))\n                {\n                    if(!file.exists(transfer.file))\n                        file.create(transfer.file)\n                }\n            )\n    }\n}(object=%o,transfer.file=\"%f\")";

fn r_devices(ext: &str) -> (&'static str, &'static str) {
    match ext {
        "bmp" => ("bmp", "filename"),
        "jpg" | "jpeg" => ("jpeg", "filename"),
        "tikz" => ("tikz", "file"),
        "tiff" => ("tiff", "filename"),
        "svg" => ("svg", "file"),
        "pdf" => ("pdf", "file"),
        "ps" | "postscript" => ("postscript", "file"),
        _ => ("png", "filename"),
    }
}

fn r(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let rp = rp(p);
    let graphics = if rp.iter().any(|r| r == "graphics") {
        core::graphical_output_file(p)?
    } else {
        None
    };
    let colnames_p = if graphics.is_none() {
        get(p, ":colnames").cloned()
    } else {
        None
    };
    let rownames_p = if graphics.is_none() {
        get(p, ":rownames").cloned()
    } else {
        None
    };
    let inside = expand(env, "R", body, p)?.unwrap_or_default();
    let fb = match &graphics {
        Some(f) => {
            let ext = Path::new(f)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("png");
            let (dev, arg) = r_devices(ext);
            const ALLOWED: &[&str] = &[
                ":width",
                ":height",
                ":bg",
                ":units",
                ":pointsize",
                ":antialias",
                ":quality",
                ":compression",
                ":res",
                ":type",
                ":family",
                ":title",
                ":fonts",
                ":version",
                ":paper",
                ":encoding",
                ":pagecentre",
                ":colormodel",
                ":useDingbats",
                ":horizontal",
            ];
            let args: String = p
                .iter()
                .filter(|(k, _)| ALLOWED.contains(&k.as_str()))
                .map(|(k, v)| format!(",{}={}", &k[1..], prin1(v)))
                .collect();
            let extra = get_str(p, ":R-dev-args");
            format!(
                "{dev}({arg}=\"{f}\"{args}{}{}); tryCatch({{\n{inside}\n}},error=function(e){{plot(x=-1:1, y=-1:1, type='n', xlab='', ylab='', axes=FALSE); text(x=0, y=0, labels=e$message, col='red'); paste('ERROR', e$message, sep=' : ')}}); dev.off()",
                if extra.is_some() { "," } else { "" },
                extra.unwrap_or_default()
            )
        }
        None => inside,
    };
    let yes = |v: &Option<Sexp>| v.as_ref().and_then(as_string).as_deref() == Some("yes");
    let colnames = yes(&colnames_p)
        || core::pick_name(get(p, ":colname-names"), colnames_p.as_ref()).is_some();
    let rownames = yes(&rownames_p)
        || core::pick_name(get(p, ":rowname-names"), rownames_p.as_ref()).is_some();
    let cmd = opt("org-babel-R-command", "R --slave --no-save");
    let session = session_name(p);
    let write_cmd = |obj: &str, file: &Path| {
        R_WRITE
            .replace("%r", if rownames { "TRUE" } else { "FALSE" })
            .replace(
                "%c",
                if colnames {
                    if rownames { "NA" } else { "TRUE" }
                } else {
                    "FALSE"
                },
            )
            .replace("%o", obj)
            .replace("%f", &file.display().to_string())
    };
    let result = if result_type(p) == "value" {
        let tmp = env.temp_file("R-", "");
        match &session {
            Some(name) => {
                let name = if name.is_empty() {
                    "*R*".to_owned()
                } else {
                    name.clone()
                };
                let kind = session::Kind::R(cmd.clone());
                session::eval(&kind, &name, &env.dir, value::chomp(&fb, value::WS))?;
                session::eval(&kind, &name, &env.dir, &write_cmd(".Last.value", &tmp))?;
            }
            None => {
                env.eval(
                    &cmd,
                    &write_cmd(&format!("{{function ()\n{{\n{fb}\n}}}}()"), &tmp),
                )?;
            }
        }
        let text = read_file(&tmp);
        let r = match result_cond(&rp) {
            None => None,
            Some(true) => Some(str(value::chomp(&text, "\n"))),
            Some(false) => Some(value::import_table(&text, Sep::Tab)?),
        };
        r.map(|r| {
            if colnames {
                match value::car_cdr(&r) {
                    Some((h, rest)) => value::cons(h, value::cons(hline(), rest)),
                    None => r,
                }
            } else {
                r
            }
        })
    } else {
        match &session {
            Some(name) => {
                let name = if name.is_empty() {
                    "*R*".to_owned()
                } else {
                    name.clone()
                };
                let src = env.temp_file("R-", "");
                write(&src, value::chomp(&fb, value::WS))?;
                let out = session::eval(
                    &session::Kind::R(cmd),
                    &name,
                    &env.dir,
                    &format!("source('{}', echo=F, print.eval=T)", src.display()),
                )?;
                Some(str(value::chomp(out.trim_end(), "\n")))
            }
            None => Some(str(env.eval(&cmd, &fb)?)),
        }
    };
    Ok(if graphics.is_some() { None } else { result })
}

// ---- julia ----

fn julia_quote(v: &Sexp) -> String {
    match v {
        Sexp::Str(s) => format!("\"{}\"", s.replace('"', "\"\"")),
        v => prin1(v),
    }
}

fn julia_assignments(p: &[(String, Sexp)]) -> Vec<String> {
    let cn = items(get(p, ":colname-names").unwrap_or(&Sexp::Nil))
        .unwrap_or(&[])
        .to_vec();
    let rn = items(get(p, ":rowname-names").unwrap_or(&Sexp::Nil))
        .unwrap_or(&[])
        .to_vec();
    params::vars(p)
        .iter()
        .enumerate()
        .map(|(i, (n, v))| {
            let c = cn.get(i).and_then(value::car_cdr).map(|(_, x)| x);
            let r = rn.get(i).and_then(value::car_cdr).map(|(_, x)| x);
            let v = core::reassemble_table(v.clone(), c, r);
            if value::listp(&v) {
                let t = if items(&v).unwrap_or(&[]).first().is_some_and(value::listp) {
                    v.clone()
                } else {
                    list(vec![v.clone()])
                };
                let csv = value::orgtbl_to_generic(&t, ",", &julia_quote, None);
                format!("{n} = begin\n    using CSV\n    CSV.read(\"{csv}\")\nend")
            } else {
                format!("{n} = {}", julia_quote(&v))
            }
        })
        .collect()
}

const JULIA_WRITE: &str = "begin\n    local p_ans = %o\n    local p_tmp_file = \"%f\"\n    try\n        using CSV, DataFrames\n        if typeof(p_ans) <: DataFrame\n           p_ans_df = p_ans\n        else\n            p_ans_df = DataFrame(:ans => p_ans)\n        end\n        CSV.write(p_tmp_file,\n                  p_ans_df,\n                  writeheader = %h,\n                  transform = (col, val) -> something(val, missing),\n                  missingstring = \"nil\",\n                  quotestrings = false)\n        p_ans\n    catch e\n        err_msg = \"Source block evaluation failed. $e\"\n        CSV.write(p_tmp_file,\n                  DataFrame(:ans => err_msg),\n                  writeheader = false,\n                  transform = (col, val) -> something(val, missing),\n                  missingstring = \"nil\",\n                  quotestrings = false)\n        err_msg\n    end\nend";

fn julia(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let rp = rp(p);
    let graphics = if rp.iter().any(|r| r == "graphics") {
        core::graphical_output_file(p)?
    } else {
        None
    };
    let colnames_p = if graphics.is_none() {
        get(p, ":colnames").cloned()
    } else {
        None
    };
    let colnames = colnames_p.as_ref().and_then(as_string).as_deref() == Some("yes")
        || core::pick_name(get(p, ":colname-names"), colnames_p.as_ref()).is_some();
    let fb = expand(env, "julia", body, p)?.unwrap_or_default();
    let cmd = opt("org-babel-julia-command", "julia");
    let session = session_name(p);
    let result = if result_type(p) == "value" {
        let tmp = env.temp_file("julia-", "");
        let w = |obj: &str| {
            JULIA_WRITE
                .replace("%o", obj)
                .replace("%f", &tmp.display().to_string())
                .replace("%h", if colnames { "true" } else { "false" })
        };
        match &session {
            Some(name) => {
                let name = if name.is_empty() {
                    "*Julia*".to_owned()
                } else {
                    name.clone()
                };
                let kind = session::Kind::Julia(cmd);
                session::eval(&kind, &name, &env.dir, value::chomp(&fb, value::WS))?;
                session::eval(&kind, &name, &env.dir, &w("ans"))?;
            }
            None => {
                env.eval(&cmd, &w(&format!("begin {fb} end")))?;
            }
        }
        let text = read_file(&tmp);
        let r = match result_cond(&rp) {
            None => None,
            Some(true) => Some(str(text)),
            Some(false) => Some(value::import_table(&text, Sep::Comma)?),
        };
        r.map(|r| {
            if colnames {
                value::car_cdr(&r).map_or(r.clone(), |(h, rest)| {
                    value::cons(h, value::cons(hline(), rest))
                })
            } else {
                r
            }
        })
    } else {
        match &session {
            Some(name) => {
                let name = if name.is_empty() {
                    "*Julia*".to_owned()
                } else {
                    name.clone()
                };
                Some(str(session::eval(
                    &session::Kind::Julia(cmd),
                    &name,
                    &env.dir,
                    &fb,
                )?
                .trim_end()
                .to_owned()))
            }
            None => Some(str(env.eval(&cmd, &fb)?)),
        }
    };
    Ok(if graphics.is_some() { None } else { result })
}

// ---- sql and sqlite ----

fn sql_expand(env: &mut Env, body: &str, p: &[(String, Sexp)], sqlite: bool) -> R<String> {
    let mut b = body.to_owned();
    for (n, v) in params::vars(p) {
        let rep = if value::listp(&v) && !v.is_nil() {
            let f = env.temp_file("sql-data-", "");
            let csv = if sqlite {
                value::orgtbl_to_generic(&v, ",", &|x: &Sexp| value::csv_field(&princ(x)), None)
            } else {
                value::orgtbl_to_generic(&v, ",", &|x: &Sexp| value::csv_field(&echo(x)), None)
            };
            write(&f, &csv)?;
            f.display().to_string()
        } else {
            echo(&v)
        };
        b = b.replace(&format!("${n}"), &rep);
    }
    let mut parts = vec![];
    parts.extend(get_str(p, ":prologue"));
    parts.push(b);
    parts.extend(get_str(p, ":epilogue"));
    Ok(parts.join("\n"))
}

fn sqlite(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let rparams: Vec<String> = get_str(p, ":results")
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let db = get_str(p, ":db");
    let separator = get_str(p, ":separator");
    let nullvalue = get_str(p, ":nullvalue");
    let headers = get_str(p, ":colnames").as_deref() == Some("yes");
    let readonly = get_str(p, ":readonly").as_deref() == Some("yes");
    let others: Vec<&str> = [
        ":header", ":echo", ":bail", ":column", ":csv", ":html", ":line", ":list",
    ]
    .into_iter()
    .filter(|k| has(p, k))
    .collect();
    let mode = others
        .iter()
        .any(|o| matches!(*o, ":csv" | ":column" | ":line" | ":list" | ":html"))
        || separator.is_some();
    let cmd = format!(
        "{} {} {} {} {} {} {} {} ",
        opt("org-babel-sqlite3-command", "sqlite3"),
        if headers { "-header" } else { "-noheader" },
        separator
            .map(|s| format!("-separator {s}"))
            .unwrap_or_default(),
        nullvalue
            .map(|s| format!("-nullvalue {s}"))
            .unwrap_or_default(),
        others
            .iter()
            .map(|o| format!("-{}", &o[1..]))
            .collect::<Vec<_>>()
            .join(" "),
        if mode { "" } else { "-csv" },
        if readonly { "-readonly" } else { "" },
        db.unwrap_or_default()
    );
    let input = sql_expand(env, body, p, true)?;
    let out = env.eval(&cmd, &input)?;
    Ok(match result_cond(&rparams) {
        None => None,
        Some(true) => Some(str(out)),
        Some(false) if out.is_empty() => Some(str("")),
        Some(false) => {
            let rows = value::convert_region(&out, if mode { Sep::Auto } else { Sep::Comma })?;
            let mut table: Vec<Sexp> = rows
                .into_iter()
                .map(|r| list(r.into_iter().map(str).collect()))
                .collect();
            if headers && !table.is_empty() {
                table.insert(1, hline());
            }
            if table.len() == 1 && items(&table[0]).is_some_and(|r| r.len() == 1) {
                let cell = items(&table[0]).unwrap()[0].clone();
                Some(value::read(&princ(&cell), true)?)
            } else {
                Some(list(
                    table
                        .into_iter()
                        .map(|r| {
                            if is_hline(&r) {
                                r
                            } else {
                                list(
                                    items(&r)
                                        .unwrap_or(&[])
                                        .iter()
                                        .map(|c| value::read(&princ(c), true).unwrap_or(c.clone()))
                                        .collect(),
                                )
                            }
                        })
                        .collect(),
                ))
            }
        }
    })
}

fn sql_param(p: &[(String, Sexp)], key: &str) -> Option<String> {
    get_str(p, key).or_else(|| {
        let conn = get_str(p, ":dbconnection")?;
        let map = [
            (":dbhost", "sql-server"),
            (":dbport", "sql-port"),
            (":dbuser", "sql-user"),
            (":dbpassword", "sql-password"),
            (":dbinstance", "sql-dbinstance"),
            (":database", "sql-database"),
        ];
        let mapped = map.iter().find(|(k, _)| *k == key)?.1;
        let alist = sexp::option("sql-connection-alist")?;
        let entry = items(&alist)?.iter().find(|e| {
            value::car_cdr(e).is_some_and(|(k, _)| princ(&k).eq_ignore_ascii_case(&conn))
        })?;
        let (_, rest) = value::car_cdr(entry)?;
        items(&rest)?
            .iter()
            .filter_map(value::car_cdr)
            .find(|(k, _)| princ(k) == mapped)
            .and_then(|(_, v)| items(&v).and_then(|x| x.first().cloned()).or(Some(v)))
            .and_then(|v| as_string(&v))
    })
}

fn sql(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let cmdline = get_str(p, ":cmdline").unwrap_or_default();
    let (host, port, user, pass, inst, db) = (
        sql_param(p, ":dbhost"),
        sql_param(p, ":dbport"),
        sql_param(p, ":dbuser"),
        sql_param(p, ":dbpassword"),
        sql_param(p, ":dbinstance"),
        sql_param(p, ":database"),
    );
    let engine = get_str(p, ":engine").unwrap_or_default();
    let colnames = get_str(p, ":colnames").as_deref() != Some("no");
    let in_file = env.temp_file("sql-in-", "");
    let out_file = get_str(p, ":out-file")
        .map(|f| env.dir.join(f))
        .unwrap_or_else(|| env.temp_file("sql-out-", ""));
    let sq = |s: &Option<String>| s.as_deref().map(core::shell_quote);
    let (fi, fo) = (q(&in_file), q(&out_file));
    let join = |v: Vec<Option<String>>| v.into_iter().flatten().collect::<Vec<_>>().join(" ");
    let command = match engine.as_str() {
        "dbi" => {
            format!("dbish --batch {cmdline} < {fi} | sed '/^+/d;s/^|//;s/(NULL)/ /g;$d' > {fo}")
        }
        "monetdb" => format!("mclient -f tab {cmdline} < {fi} > {fo}"),
        "mssql" => format!(
            "sqlcmd {cmdline} -s \"\t\" {} -i \"{}\" -o \"{}\"",
            join(vec![
                sq(&host).map(|h| format!("-S {h}")),
                sq(&user).map(|u| format!("-U {u}")),
                sq(&pass).map(|x| format!("-P {x}")),
                sq(&db).map(|d| format!("-d {d}"))
            ]),
            in_file.display(),
            out_file.display()
        ),
        "mysql" => format!(
            "mysql {} {} {cmdline} < {fi} > {fo}",
            join(vec![
                sq(&host).map(|h| format!("-h{h}")),
                port.as_ref().map(|x| format!("-P{x}")),
                sq(&user).map(|u| format!("-u{u}")),
                sq(&pass).map(|x| format!("-p{x}")),
                sq(&db).map(|d| format!("-D{d}"))
            ]),
            if colnames { "" } else { "-N" }
        ),
        "postgresql" | "postgres" => format!(
            "{}{} --set=\"ON_ERROR_STOP=1\" {} -A -P footer=off -F \"\t\"  {} -f {fi} -o {fo} {cmdline}",
            pass.as_ref()
                .map(|x| format!("PGPASSWORD={} ", core::shell_quote(x)))
                .unwrap_or_default(),
            opt("sql-postgres-program", "psql"),
            if colnames { "" } else { "-t" },
            join(vec![
                sq(&host).map(|h| format!("-h{h}")),
                port.as_ref().map(|x| format!("-p{x}")),
                sq(&user).map(|u| format!("-U{u}")),
                sq(&db).map(|d| format!("-d{d}"))
            ])
        ),
        "sqsh" => format!(
            "sqsh {cmdline} {} -i \"{}\" -o \"{}\" -m csv",
            join(vec![
                sq(&host).map(|h| format!("-S {h}")),
                sq(&user).map(|u| format!("-U {u}")),
                sq(&pass).map(|x| format!("-P {x}")),
                sq(&db).map(|d| format!("-D {d}"))
            ]),
            in_file.display(),
            out_file.display()
        ),
        "vertica" => format!(
            "vsql {} -f {fi} -o {fo} {cmdline}",
            join(vec![
                sq(&host).map(|h| format!("-h {h}")),
                port.as_ref().map(|x| format!("-p {x}")),
                sq(&user).map(|u| format!("-U {u}")),
                sq(&pass).map(|x| format!("-w {x}")),
                sq(&db).map(|d| format!("-d {d}"))
            ])
        ),
        "oracle" => {
            let conn = match (&user, &pass, &db, &host, &port) {
                (Some(u), Some(pw), Some(d), Some(h), Some(po)) => format!("{u}/{pw}@{h}:{po}/{d}"),
                (Some(u), Some(pw), Some(d), _, _) => format!("{u}/{pw}@{d}"),
                _ => return err("Missing information to connect to database"),
            };
            format!("sqlplus -s {conn} < {fi} > {fo}")
        }
        "saphana" => format!(
            "hdbsql {} -I {fi} -o {fo} {cmdline}",
            join(vec![
                host.as_ref().map(|h| match &port {
                    Some(po) => format!("-n {}:{po}", core::shell_quote(h)),
                    None => format!("-n {}", core::shell_quote(h)),
                }),
                inst.as_ref().map(|i| format!("-i {i}")),
                sq(&user).map(|u| format!("-u {u}")),
                sq(&pass).map(|x| format!("-p {x}")),
                sq(&db).map(|d| format!("-d {d}")),
            ])
        ),
        e => return err(format!("No support for the {e} SQL engine")),
    };
    let header = match engine.as_str() {
        "dbi" => "/format partbox\n",
        "oracle" => {
            "SET PAGESIZE 50000\nSET NEWPAGE 0\nSET TAB OFF\nSET SPACE 0\nSET LINESIZE 9999\nSET TRIMOUT ON TRIMSPOOL ON\nSET ECHO OFF\nSET FEEDBACK OFF\nSET VERIFY OFF\nSET HEADING ON\nSET MARKUP HTML OFF SPOOL OFF\nSET COLSEP '|'\n"
        }
        "mssql" | "sqsh" => "SET NOCOUNT ON\n",
        "vertica" => "\\a\n",
        _ => "",
    };
    write(
        &in_file,
        &format!(
            "{header}{}{}",
            sql_expand(env, body, p, false)?,
            if engine == "sqsh" { "\ngo" } else { "" }
        ),
    )?;
    env.eval(&command, "")?;
    let text = read_file(&out_file);
    let r = match result_cond(&rp(p)) {
        None => return Ok(None),
        Some(true) => return Ok(Some(str(text))),
        Some(false) => {
            let mut text = text;
            let mut delim = String::new();
            if [
                "dbi",
                "mysql",
                "postgresql",
                "postgres",
                "saphana",
                "sqsh",
                "vertica",
            ]
            .contains(&engine.as_str())
            {
                if colnames && let Some(i) = text.find('\n') {
                    text.insert_str(i + 1, "-\n");
                    delim = "-".into();
                }
            } else {
                if let Some(l) = text.lines().find(|l| {
                    l.starts_with('-') && l.trim_start_matches('-').chars().next().is_some()
                }) {
                    delim = l.chars().take_while(|&c| c == '-').collect();
                }
                text = text.trim_end_matches('\n').to_owned();
            }
            let sep = if matches!(engine.as_str(), "saphana" | "sqsh") {
                Sep::Comma
            } else {
                Sep::Tab
            };
            let rows = value::convert_region(&text, sep)?;
            list(
                rows.into_iter()
                    .map(|r| {
                        if r.first().is_some_and(|c| *c == delim) && !delim.is_empty() {
                            hline()
                        } else {
                            list(r.into_iter().map(str).collect())
                        }
                    })
                    .collect(),
            )
        }
    };
    Ok(Some(reassemble(r, p)))
}

// ---- graphics and documents ----

fn ext_of(f: &str) -> String {
    Path::new(f)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_owned()
}

fn dot(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let out = get_str(p, ":file").ok_or("You need to specify a :file parameter")?;
    let cmdline = get_str(p, ":cmdline").unwrap_or_else(|| format!("-T{}", ext_of(&out)));
    let cmd = get_str(p, ":cmd").unwrap_or_else(|| "dot".into());
    let input = env.temp_file("dot-", "");
    let b = expand(env, "dot", body, p)?.unwrap_or_default();
    write(&input, &b)?;
    env.eval(
        &format!(
            "{cmd} {} {cmdline} -o {}",
            q(&input),
            q(&env.dir.join(&out))
        ),
        "",
    )?;
    Ok(None)
}

fn latex_header(p: &[(String, Sexp)]) -> String {
    let mut packages: Vec<String> = vec![];
    if let Some(v) = get(p, ":packages") {
        packages.extend(items(v).unwrap_or(&[]).iter().map(latex_package));
    }
    if let Some(v) = sexp::option("org-latex-packages-alist") {
        packages.extend(items(&v).unwrap_or(&[]).iter().map(latex_package));
    }
    let tpl = opt(
        "org-format-latex-header",
        "\\documentclass{article}\n\\usepackage[usenames]{color}\n[DEFAULT-PACKAGES]\n[PACKAGES]\n\\pagestyle{empty}             % do not remove\n% The settings below are copied from fullpage.sty\n\\setlength{\\textwidth}{\\paperwidth}\n\\addtolength{\\textwidth}{-3cm}\n\\setlength{\\oddsidemargin}{1.5cm}\n\\addtolength{\\oddsidemargin}{-2.54cm}\n\\setlength{\\evensidemargin}{\\oddsidemargin}\n\\setlength{\\textheight}{\\paperheight}\n\\addtolength{\\textheight}{-\\headheight}\n\\addtolength{\\textheight}{-\\headsep}\n\\addtolength{\\textheight}{-\\footskip}\n\\addtolength{\\textheight}{-3cm}\n\\setlength{\\topmargin}{1.5cm}\n\\addtolength{\\topmargin}{-2.54cm}",
    );
    tpl.replace(
        "[DEFAULT-PACKAGES]",
        "\\usepackage[T1]{fontenc}\n\\usepackage{graphicx}\n\\usepackage{longtable}\n\\usepackage{wrapfig}\n\\usepackage{rotating}\n\\usepackage[normalem]{ulem}\n\\usepackage{amsmath}\n\\usepackage{amssymb}\n\\usepackage{capt-of}",
    )
    .replace("[PACKAGES]", &packages.join("\n"))
}

fn latex_package(v: &Sexp) -> String {
    match items(v) {
        Some(x) if value::listp(v) && !v.is_nil() => {
            let opts = x.first().map(princ).unwrap_or_default();
            let name = x.get(1).map(princ).unwrap_or_default();
            if opts.is_empty() {
                format!("\\usepackage{{{name}}}")
            } else {
                format!("\\usepackage[{opts}]{{{name}}}")
            }
        }
        _ => format!("\\usepackage{{{}}}", princ(v)),
    }
}

fn headers_text(p: &[(String, Sexp)]) -> String {
    match get(p, ":headers") {
        Some(v) if value::listp(v) && !v.is_nil() => items(v)
            .unwrap_or(&[])
            .iter()
            .map(princ)
            .collect::<Vec<_>>()
            .join("\n"),
        Some(v) => princ(v),
        None => String::new(),
    }
}

/// org-latex-compile: run org-latex-pdf-process (or pdflatex) on TEX.
fn latex_compile(env: &mut Env, tex: &Path) -> R<std::path::PathBuf> {
    let dir = tex.parent().unwrap_or(Path::new(".")).to_path_buf();
    let base = tex
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("x")
        .to_owned();
    let procs = options::strings(
        "org-latex-pdf-process",
        &["pdflatex -interaction nonstopmode -output-directory %o %f"],
    );
    for cmd in procs {
        let c = cmd
            .replace("%latex", "pdflatex")
            .replace("%bib", "bibtex")
            .replace("%o", &q(&dir))
            .replace("%f", &q(tex))
            .replace("%F", &q(tex))
            .replace("%b", &base);
        let (_, _, _) = core::run_shell(&c, "", &dir)?;
    }
    let pdf = dir.join(format!("{base}.pdf"));
    if !pdf.exists() {
        env.cx.message("org babel latex failed");
        return err(format!("PDF file {} wasn't produced", pdf.display()));
    }
    Ok(pdf)
}

fn latex(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let body = expand(env, "latex", body, p)?.unwrap_or_default();
    let Some(out) = get_str(p, ":file") else {
        return Ok(Some(str(body)));
    };
    let out_path = env.dir.join(&out);
    let ext = ext_of(&out);
    let tex = env.temp_file("latex-", ".tex");
    let border = get_str(p, ":border");
    let imagemagick = get(p, ":imagemagick")
        .is_some_and(|v| v.truthy() && as_string(v).as_deref() != Some("nil"));
    let fit = get(p, ":fit").is_some_and(Sexp::truthy) || border.is_some();
    let headers = headers_text(p);
    if out.ends_with(".png") && !imagemagick {
        let doc = format!(
            "{}\n{headers}\n\\begin{{document}}\n{body}\n\\end{{document}}\n",
            latex_header(p)
        );
        write(&tex, &doc)?;
        let dir = tex.parent().unwrap().to_path_buf();
        let base = tex
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("x")
            .to_owned();
        core::run_shell(
            &format!(
                "latex -interaction nonstopmode -output-directory {} {}",
                q(&dir),
                q(&tex)
            ),
            "",
            &dir,
        )?;
        let dvi = dir.join(format!("{base}.dvi"));
        let dpi = 140;
        env.eval(
            &format!("dvipng -D {dpi} -T tight -o {} {}", q(&out_path), q(&dvi)),
            "",
        )?;
        return Ok(None);
    }
    if ext == "svg" {
        write(
            &tex,
            &format!(
                "\\documentclass[preview]{{standalone}}\n{headers}\\begin{{document}}{body}\\end{{document}}"
            ),
        )?;
        let pdf = latex_compile(env, &tex)?;
        let cmd = opt(
            "org-babel-latex-pdf-svg-process",
            "inkscape --pdf-poppler --export-area-drawing --export-text-to-path --export-plain-svg --export-filename=%O %f",
        );
        env.eval(
            &cmd.replace("%O", &q(&out_path)).replace("%f", &q(&pdf)),
            "",
        )?;
        return Ok(None);
    }
    if out.ends_with(".tikz") {
        write(&out_path, &body)?;
        return Ok(None);
    }
    if ext == "html" && which(&opt("org-babel-latex-htlatex", "htlatex")) {
        let pkgs = options::strings(
            "org-babel-latex-htlatex-packages",
            &[
                "[usenames]{color}",
                "{tikz}",
                "{color}",
                "{listings}",
                "{amsmath}",
            ],
        );
        let doc = format!(
            "\\documentclass[preview]{{standalone}}\n\\def\\pgfsysdriver{{pgfsys-tex4ht.def}}\n{}{}\\begin{{document}}{body}\\end{{document}}",
            pkgs.iter()
                .map(|p| format!("\\usepackage{p}"))
                .collect::<Vec<_>>()
                .join("\n"),
            if headers.is_empty() {
                String::new()
            } else {
                format!("\n{headers}\n")
            }
        );
        write(&tex, &doc)?;
        let dir = tex.parent().unwrap().to_path_buf();
        core::run_shell(
            &format!("{} {}", opt("org-babel-latex-htlatex", "htlatex"), q(&tex)),
            "",
            &dir,
        )?;
        let html = tex.with_extension("html");
        let svg = dir.join(format!(
            "{}-1.svg",
            tex.file_stem().and_then(|s| s.to_str()).unwrap_or("")
        ));
        if svg.exists() {
            if !out.ends_with(".svg") {
                return err("SVG file produced but HTML file requested");
            }
            std::fs::rename(svg, &out_path).map_err(|e| e.to_string())?;
        } else if html.exists() {
            std::fs::rename(html, &out_path).map_err(|e| e.to_string())?;
        }
        return Ok(None);
    }
    if ext == "pdf" || imagemagick {
        let height = if fit { get_str(p, ":pdfheight") } else { None };
        let width = if fit { get_str(p, ":pdfwidth") } else { None };
        let mut doc = latex_header(p);
        if fit {
            doc.push_str("\n\\usepackage[active, tightpage]{preview}\n");
        }
        if let Some(b) = &border {
            doc.push_str(&format!("\\setlength{{\\PreviewBorder}}{{{b}}}"));
        }
        if let Some(h) = height {
            doc.push_str(&format!("\n\\pdfpageheight {h}"));
        }
        if let Some(w) = width {
            doc.push_str(&format!("\n\\pdfpagewidth {w}"));
        }
        if !headers.is_empty() {
            doc.push_str(&format!("\n{headers}\n"));
        }
        doc.push_str(&if fit {
            format!("\n\\begin{{document}}\n\\begin{{preview}}\n{body}\n\\end{{preview}}\n\\end{{document}}\n")
        } else {
            format!("\n\\begin{{document}}\n{body}\n\\end{{document}}\n")
        });
        write(&tex, &doc)?;
        let _ = std::fs::remove_file(&out_path);
        let pdf = latex_compile(env, &tex)?;
        if ext == "pdf" {
            std::fs::rename(&pdf, &out_path)
                .or_else(|_| std::fs::copy(&pdf, &out_path).map(|_| ()))
                .map_err(|e| e.to_string())?;
        } else {
            let cmd = format!(
                "convert {} {} {} {}",
                get_str(p, ":iminoptions").unwrap_or_default(),
                q(&pdf),
                get_str(p, ":imoutoptions").unwrap_or_default(),
                q(&out_path)
            );
            core::run_shell(&cmd, "", &env.dir)?;
        }
        return Ok(None);
    }
    err(format!(
        "Can not create {ext} files, please specify a .png or .pdf file or try the :imagemagick header argument"
    ))
}

/// executable-find.
pub fn which(cmd: &str) -> bool {
    let name = cmd.split_whitespace().next().unwrap_or(cmd);
    if name.contains('/') {
        return Path::new(name).is_file();
    }
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(name).is_file()))
}

fn ditaa(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let out = core::graphical_output_file(p)?
        .ok_or("No :file header argument given; cannot create graphical result")?;
    let sfx = ext_of(&out);
    let legacy_eps = get(p, ":eps").is_some_and(Sexp::truthy);
    let legacy_pdf = get(p, ":pdf").is_some_and(Sexp::truthy);
    if legacy_eps && legacy_pdf {
        return err("(ob-ditaa) Both :eps and :pdf legacy output types specified");
    }
    let legacy = legacy_eps || legacy_pdf;
    let eps = legacy_eps || sfx == "eps";
    let pdf = legacy_pdf || sfx == "pdf";
    let svg = !legacy && sfx == "svg";
    let png = !legacy && (sfx == "png" || !(svg || pdf || eps));
    if !(eps || pdf || svg || png) {
        return err(format!("(ob-ditaa) Unknown output file extension: {sfx}"));
    }
    let use_eps_jar = eps || pdf;
    let exec = if opt("org-ditaa-default-exec-mode", "jar") == "jar" || use_eps_jar {
        let jar = if use_eps_jar {
            opt("org-ditaa-eps-jar-path", "DitaaEps.jar")
        } else {
            opt("org-ditaa-jar-path", "ditaa.jar")
        };
        let jar = options::expand(&jar);
        if !jar.exists() {
            return err(format!(
                "(ob-ditaa) Could not find jar file {}",
                jar.display()
            ));
        }
        format!(
            "{}{} -jar {}",
            opt("org-ditaa-java-exec", "java"),
            get_str(p, ":java")
                .map(|j| format!(" {j}"))
                .unwrap_or_default(),
            q(&jar)
        )
    } else {
        opt("org-ditaa-exec", "ditaa")
    };
    let input = env.temp_file("ditaa-", "");
    let out_path = env.dir.join(&out);
    let ditaa_out = if pdf {
        format!("{}.eps", input.display())
    } else {
        out_path.display().to_string()
    };
    write(&input, body)?;
    let cmd = format!(
        "{exec}{}{} -e utf-8 {} {}",
        get_str(p, ":cmdline")
            .map(|c| format!(" {c}"))
            .unwrap_or_default(),
        if svg { " --svg" } else { "" },
        q(&input),
        core::shell_quote(&ditaa_out)
    );
    core::run_shell(&cmd, "", &env.dir)?;
    if pdf {
        core::run_shell(
            &format!(
                "epstopdf {} -o={}",
                core::shell_quote(&ditaa_out),
                q(&out_path)
            ),
            "",
            &env.dir,
        )?;
    }
    Ok(None)
}

fn plantuml(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let do_export = rp(p).iter().any(|r| r == "file");
    let out = if do_export {
        env.dir.join(get_str(p, ":file").ok_or("No :file provided but :results set to file.  For plain text output, set :results to verbatim")?)
    } else {
        env.temp_file("plantuml-", ".txt")
    };
    let mode = opt("org-plantuml-exec-mode", "jar");
    let args = options::strings("org-plantuml-args", &["-headless"]);
    let (exe, exe_args) = if mode == "plantuml" {
        (opt("org-plantuml-executable-path", "plantuml"), args)
    } else {
        let jar = opt("org-plantuml-jar-path", "");
        if jar.is_empty() {
            return err("`org-plantuml-jar-path' is not set");
        }
        let jar = options::expand(&jar);
        if !jar.exists() {
            return err(format!("Could not find plantuml.jar at {}", jar.display()));
        }
        let mut a = vec![
            get_str(p, ":java").unwrap_or_default(),
            "-jar".into(),
            q(&jar),
        ];
        a.extend(args);
        ("java".into(), a)
    };
    // The body between @startX and @endX, variables expanded inside.
    let re = regex::Regex::new(
        r"(?s)^(.*?)(?m:^)[ \t]*@start(\S+)[ \t]*\n?(.*?)(?m:^)[ \t]*@end(\S+)[ \t]*(?:\n(.*))?$",
    )
    .unwrap();
    let (pre, kw, inner, post) = match re.captures(body).filter(|c| c[2] == c[4]) {
        Some(c) => (
            c[1].trim().to_owned(),
            c[2].to_owned(),
            c[3].trim().to_owned(),
            c.get(5).map_or("", |m| m.as_str()).trim().to_owned(),
        ),
        None => (
            String::new(),
            "uml".into(),
            body.trim().to_owned(),
            String::new(),
        ),
    };
    let vars = assignments(env, "plantuml", p)?.unwrap_or_default();
    let full_body = [
        pre,
        format!("@start{kw}"),
        expand_generic(&inner, p, &vars),
        format!("@end{kw}"),
        post,
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join("\n");
    let input = env.temp_file("plantuml-", "");
    write(&input, &full_body)?;
    let t = match ext_of(&out.display().to_string()).as_str() {
        "png" => "-tpng",
        "svg" => "-tsvg",
        "eps" => "-teps",
        "pdf" => "-tpdf",
        "tex" => "-tlatex",
        "tikz" => "-tlatex:nopreamble",
        "vdx" => "-tvdx",
        "xmi" => "-txmi",
        "scxml" => "-tscxml",
        "html" => "-thtml",
        "txt" => "-ttxt",
        "utxt" => "-utxt",
        _ => "",
    };
    let cmd = format!(
        "{exe} {} {t} -p {} < {} > {}",
        exe_args.join(" "),
        get_str(p, ":cmdline").unwrap_or_default(),
        q(&input),
        q(&out)
    );
    env.cx.message(cmd.clone());
    env.eval(&cmd, "")?;
    if ext_of(&out.display().to_string()) == "svg"
        && options::bool("org-babel-plantuml-svg-text-to-path", false)
    {
        env.eval(&format!("inkscape {} -T -l {}", q(&out), q(&out)), "")?;
    }
    Ok(if do_export {
        None
    } else {
        Some(str(read_file(&out)))
    })
}

fn lilypond(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    if options::bool("org-babel-lilypond-arrange-mode", false) {
        let doc = env.doc.clone();
        let mut d = doc;
        super::tangle::tangle(
            &mut d,
            env.cx,
            super::tangle::Scope::All,
            Some("yes"),
            Some("lilypond"),
        )?;
        let path = d.path.clone().ok_or("Error: Tangle Failed!")?;
        let tangled = path.with_extension("lilypond");
        let ly = path.with_extension("ly");
        if !tangled.exists() {
            return err("Error: Tangle Failed!");
        }
        let _ = std::fs::remove_file(&ly);
        std::fs::rename(&tangled, &ly).map_err(|e| e.to_string())?;
        let cmds = options::strings("org-babel-lilypond-commands", &["lilypond", "open", "open"]);
        let mut args = vec![];
        for (o, a) in [
            ("org-babel-lilypond-gen-png", "--png"),
            ("org-babel-lilypond-gen-html", "--html"),
            ("org-babel-lilypond-gen-pdf", "--pdf"),
            ("org-babel-lilypond-use-eps", "-dbackend=eps"),
            ("org-babel-lilypond-gen-svg", "-dbackend=svg"),
        ] {
            if options::bool(o, false) {
                args.push(a.to_owned());
            }
        }
        args.push(format!(
            "--output={}",
            core::shell_quote(&ly.with_extension("").display().to_string())
        ));
        args.push(q(&ly));
        let (out, errs, _) = core::run_shell(
            &format!("{} {}", core::shell_quote(&cmds[0]), args.join(" ")),
            "",
            &env.dir,
        )?;
        if format!("{out}{errs}").contains("error:") {
            return err("Error in Compilation!");
        }
        return Ok(None);
    }
    let out = get_str(p, ":file").ok_or("You need to specify a :file parameter")?;
    let ft = ext_of(&out);
    let input = env.temp_file("lilypond-", "");
    let paper = format!(
        "#(if (ly:get-option 'use-paper-size-for-page)\n            (begin (ly:set-option 'use-paper-size-for-page #f)\n                   (ly:set-option 'tall-page-formats '{ft})))\n\\paper {{\n  indent=0\\mm\n  tagline=\"\"\n  oddFooterMarkup=##f\n  oddHeaderMarkup=##f\n  bookTitleMarkup=##f\n  scoreTitleMarkup=##f\n}}\n"
    );
    write(&input, &format!("{paper}{}", expand_generic(body, p, &[])))?;
    let cmds = options::strings("org-babel-lilypond-commands", &["lilypond", "open", "open"]);
    let out_base = env.dir.join(&out).with_extension("");
    env.eval(
        &format!(
            "{} -dbackend=eps -dno-gs-load-fonts -dinclude-eps-fonts {}--output={} {}{}",
            cmds[0],
            match ft.as_str() {
                "pdf" => "--pdf ",
                "eps" => "--eps ",
                _ => "--png ",
            },
            q(&out_base),
            get_str(p, ":cmdline").unwrap_or_default(),
            input.display()
        ),
        "",
    )?;
    Ok(None)
}

fn gnuplot_vars(env: &mut Env, p: &[(String, Sexp)]) -> R<Vec<(String, String)>> {
    let missing = get_str(p, ":missing");
    let mut out = vec![];
    for (n, v) in params::vars(p) {
        if value::listp(&v) && !v.is_nil() {
            let x = items(&v).unwrap_or(&[]);
            let table = if x
                .first()
                .is_some_and(|f| value::listp(f) || matches!(f, Sexp::Sym(_)))
            {
                v.clone()
            } else {
                list(x.iter().map(|e| list(vec![e.clone()])).collect())
            };
            let file = core::temp_dir().join(format!(
                "gnuplot-{}",
                &super::sha1::hex(prin1(&v).as_bytes())[..12]
            ));
            let quote = |c: &Sexp| -> String {
                let s = princ(c);
                if is_table_number(&s) {
                    s
                } else if s.is_empty() {
                    missing.clone().unwrap_or(s)
                } else if s.contains([' ', '"']) {
                    format!("\"{}\"", s.replace('"', "\"\""))
                } else {
                    s
                }
            };
            write(&file, &value::orgtbl_to_generic(&table, "\t", &quote, None))?;
            out.push((n, file.display().to_string()));
        } else {
            out.push((n, princ(&v)));
        }
    }
    let _ = env;
    Ok(out)
}

fn is_table_number(s: &str) -> bool {
    !s.is_empty() && value::read_number(s).is_some()
}

fn gnuplot_expand(env: &mut Env, body: &str, p: &[(String, Sexp)]) -> R<String> {
    let vars = gnuplot_vars(env, p)?;
    let out = get_str(p, ":file");
    let term = get_str(p, ":term").or_else(|| {
        out.as_ref().map(|f| {
            let e = ext_of(f).to_ascii_lowercase();
            if e == "eps" {
                "postscript eps".into()
            } else {
                e
            }
        })
    });
    let mut body = body.to_owned();
    let add = |t: String, body: &mut String| *body = format!("{t}\n{body}");
    if let Some(m) = get_str(p, ":missing") {
        add(format!("set datafile missing '{m}'"), &mut body);
    }
    if let Some(t) = get_str(p, ":title") {
        add(format!("set title '{t}'"), &mut body);
    }
    if let Some(l) = get(p, ":line") {
        for e in items(l).unwrap_or(std::slice::from_ref(l)) {
            add(princ(e), &mut body);
        }
    }
    if let Some(s) = get(p, ":set") {
        for e in items(s).unwrap_or(std::slice::from_ref(s)) {
            add(format!("set {}", princ(e)), &mut body);
        }
    }
    for (key, axis) in [(":xlabels", "xtics"), (":ylabels", "ytics")] {
        if let Some(v) = get(p, key) {
            let labels: Vec<String> = items(v)
                .unwrap_or(&[])
                .iter()
                .filter_map(value::car_cdr)
                .map(|(a, b)| format!("\"{}\" {}", princ(&b), princ(&a)))
                .collect();
            add(format!("set {axis} ({})", labels.join(", ")), &mut body);
        }
    }
    let timefmt = get_str(p, ":timefmt");
    if get(p, ":timeind").is_some() || timefmt.is_some() {
        add("set xdata time".into(), &mut body);
        add(
            format!(
                "set timefmt \"{}\"",
                timefmt.unwrap_or_else(|| "%Y-%m-%d-%H:%M:%S".into())
            ),
            &mut body,
        );
    }
    if let Some(f) = &out {
        add(format!("set output \"{f}\""), &mut body);
        body.push_str("\nset output\n");
    }
    if let Some(t) = term {
        add(format!("set term {t}"), &mut body);
    }
    add(
        vars.iter()
            .map(|(n, v)| format!("{n} = \"{v}\""))
            .collect::<Vec<_>>()
            .join("\n"),
        &mut body,
    );
    for (n, v) in &vars {
        body = body.replace(&format!("${n}"), v);
    }
    if let Some(pro) = get_str(p, ":prologue") {
        add(pro, &mut body);
    }
    if let Some(epi) = get_str(p, ":epilogue") {
        body = format!("{body}\n{epi}");
    }
    add(format!("cd '{}'", env.dir.display()), &mut body);
    Ok(body)
}

fn gnuplot(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let rt = get_str(p, ":results").unwrap_or_default();
    let body = gnuplot_expand(env, body, p)?;
    let output = match session_name(p) {
        Some(name) => session::eval(
            &session::Kind::Plain(
                vec!["gnuplot".into()],
                "print \"org_babel_eoe\"".into(),
                "org_babel_eoe".into(),
            ),
            &name,
            &env.dir,
            &body,
        )?,
        None => {
            let script = env.temp_file("gnuplot-script-", "");
            write(&script, &format!("{body}\n"))?;
            let (o, e, _) = core::run_shell(&format!("gnuplot {}", q(&script)), "", &env.dir)?;
            o + &e
        }
    };
    Ok(rt
        .split_whitespace()
        .any(|r| r == "output")
        .then(|| str(output)))
}

fn maxima_var(v: &Sexp) -> String {
    match items(v).filter(|_| value::listp(v)) {
        Some(x) => format!(
            "[{}]",
            x.iter().map(maxima_var).collect::<Vec<_>>().join(", ")
        ),
        None => princ(v),
    }
}

fn maxima_expand(body: &str, p: &[(String, Sexp)]) -> String {
    let graphic = core::graphical_output_file(p).ok().flatten();
    let mut parts = vec![];
    parts.extend(get_str(p, ":prologue"));
    if let Some(g) = &graphic {
        let pkg = get_str(p, ":graphics-pkg").unwrap_or_else(|| "plot".into());
        let term = ext_of(g);
        parts.push(if pkg == "draw" {
            format!("(load(draw), set_draw_defaults(terminal='{term},file_name={}))$", prin1(&str(Path::new(g).with_extension("").display().to_string())))
        } else {
            format!("(set_plot_option ('[gnuplot_term, {term}]), set_plot_option ('[gnuplot_out_file, {}]))$", prin1(&str(g.clone())))
        });
    }
    parts.push(
        params::vars(p)
            .iter()
            .map(|(n, v)| format!("{n}: {}$", maxima_var(v)))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    parts.push(body.to_owned());
    parts.extend(get_str(p, ":epilogue"));
    parts.push(if graphic.is_some() {
        "gnuplot_close ()$".into()
    } else {
        String::new()
    });
    parts.join("\n")
}

fn maxima(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let rparams: Vec<String> = get_str(p, ":results")
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let cmdline = get_str(p, ":cmdline").unwrap_or_default();
    let batch = get_str(p, ":batch").unwrap_or_else(|| "batchload".into());
    let cmdline = if cmdline.is_empty() || batch == "batchload" {
        format!("{cmdline} --very-quiet")
    } else {
        cmdline
    };
    let input = env.temp_file("maxima-", ".max");
    write(&input, &maxima_expand(body, p))?;
    let cmd = format!(
        "{} -r {} {cmdline}",
        opt("org-babel-maxima-command", "maxima"),
        core::shell_quote(&format!(
            "(linenum:0, {batch}({}))$",
            prin1(&str(input.display().to_string()))
        ))
    );
    let raw = env.eval(&cmd, "")?;
    let filters = [
        r"\(linenum:0,$",
        "batch",
        "^rat: replaced .*$",
        "^;;; Loading #P",
        "^read and interpret",
        r"^\(%(i-?[0-9]+)\)[ ]$",
        r"^Loading .+maxima-init\.mac",
    ];
    let res: Vec<&str> = raw
        .split(['\r', '\n'])
        .filter(|l| {
            !l.is_empty()
                && !filters
                    .iter()
                    .any(|f| regex::Regex::new(f).unwrap().is_match(l))
        })
        .collect();
    let result = res.join("\n");
    if core::graphical_output_file(p).ok().flatten().is_some() {
        return Ok(None);
    }
    Ok(match result_cond(&rparams) {
        None => None,
        Some(true) => Some(str(result)),
        Some(false) => Some(value::import_table(&result, Sep::Auto)?),
    })
}

fn octave_var(v: &Sexp) -> String {
    match items(v).filter(|_| value::listp(v)) {
        Some(x) => {
            let sep = if x.first().is_some_and(value::listp) {
                "; "
            } else {
                ","
            };
            format!(
                "[{}]",
                x.iter().map(octave_var).collect::<Vec<_>>().join(sep)
            )
        }
        None => match v {
            Sexp::Str(s) => format!("'{s}'"),
            v => princ(v),
        },
    }
}

fn octave_import(text: &str) -> R<Sexp> {
    // Drop leading blank and comment lines.
    let lines: Vec<&str> = text.split('\n').collect();
    let start = lines.iter().position(|l| {
        let t = l.trim_start();
        !t.is_empty() && !t.starts_with('#')
    });
    let rest = match start {
        Some(i) => lines[i..].join("\n"),
        None => String::new(),
    };
    Ok(value::import_table(&rest, Sep::Tab)?)
}

fn octave(env: &mut Env, body: &str, p: &Params, matlab: bool) -> R<Option<Sexp>> {
    let fb = full(env, "octave", body, p)?;
    let gfx = core::graphical_output_file(p).ok().flatten();
    let code = match &gfx {
        Some(g) => format!(
            "set (0, \"defaultfigurevisible\", \"off\");\n{fb}\nprint -dpng {}\nans={}",
            prin1(&str(g.clone())),
            prin1(&str(g.clone()))
        ),
        None => fb,
    };
    let cmd = if matlab {
        opt("org-babel-matlab-shell-command", "matlab -nosplash")
    } else {
        opt("org-babel-octave-shell-command", "octave -q")
    };
    let wrapper = "%s\nif ischar(ans), fid = fopen('%f', 'w'); fdisp(fid, ans); fclose(fid);\nelse, dlmwrite('%f', ans, '\\t')\nend";
    let result = match session_name(p) {
        Some(name) => {
            let name = if name.is_empty() {
                if matlab {
                    "*Inferior Matlab*"
                } else {
                    "*Inferior Octave*"
                }
                .to_owned()
            } else {
                name
            };
            let kind = session::Kind::Plain(
                cmd.split_whitespace().map(str::to_owned).collect(),
                "disp('org_babel_eoe')".into(),
                "org_babel_eoe".into(),
            );
            if result_type(p) == "value" {
                let tmp = env.temp_file(if matlab { "matlab-" } else { "octave-" }, "");
                session::eval(
                    &kind,
                    &name,
                    &env.dir,
                    &wrapper
                        .replace("%s", &code)
                        .replace("%f", &tmp.display().to_string()),
                )?;
                octave_import(&read_file(&tmp))?
            } else {
                let out = session::eval(&kind, &name, &env.dir, &code)?;
                str(out
                    .lines()
                    .map(|l| value::trim(l).trim_matches('"'))
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
        }
        None => {
            if result_type(p) == "output" {
                str(env.eval(&cmd, &code)?)
            } else {
                let tmp = env.temp_file("octave-", "");
                env.eval(
                    &cmd,
                    &wrapper
                        .replace("%s", &code)
                        .replace("%f", &tmp.display().to_string()),
                )?;
                octave_import(&read_file(&tmp))?
            }
        }
    };
    Ok(if gfx.is_some() {
        None
    } else {
        Some(reassemble(result, p))
    })
}

// ---- compiled languages ----

#[derive(Clone, Copy, PartialEq, Eq)]
enum Variant {
    C,
    Cpp,
    D,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Base {
    Int,
    Float,
    Str,
}

fn base_type(v: &Sexp) -> Option<Base> {
    match v {
        Sexp::Int(_) => Some(Base::Int),
        Sexp::Float(_) => Some(Base::Float),
        Sexp::List(x) | Sexp::Vector(x) => {
            let mut t = None;
            for e in x {
                match base_type(e) {
                    Some(Base::Str) => t = Some(Base::Str),
                    Some(Base::Float) if t != Some(Base::Str) => t = Some(Base::Float),
                    Some(Base::Int) if t.is_none() => t = Some(Base::Int),
                    _ => {}
                }
            }
            t
        }
        Sexp::Nil => None,
        _ => Some(Base::Str),
    }
}

fn c_fmt(b: Option<Base>, v: &Sexp) -> String {
    match b {
        Some(Base::Str) | None => format!("\"{}\"", princ(v)),
        _ => princ(v),
    }
}

fn c_var(var: &str, val: &Sexp, variant: Variant) -> String {
    let val = match val {
        Sexp::Sym(s) if s.chars().count() == 1 => Sexp::Int(s.chars().next().unwrap() as i64),
        Sexp::Sym(s) => str(s.clone()),
        v => v.clone(),
    };
    let b = base_type(&val);
    let ty = match b {
        Some(Base::Int) => "int",
        Some(Base::Float) => "double",
        _ => {
            if variant == Variant::D {
                "string"
            } else {
                "const char*"
            }
        }
    };
    let (suffix, data) = match &val {
        Sexp::Int(_) | Sexp::Float(_) => (String::new(), princ(&val)),
        Sexp::List(x) if x.first().is_some_and(|f| matches!(f, Sexp::List(_))) => {
            let ncols = items(&x[0]).map_or(0, <[Sexp]>::len);
            let rows: Vec<String> = x
                .iter()
                .map(|r| {
                    let cells = items(r)
                        .unwrap_or(&[])
                        .iter()
                        .map(|w| c_fmt(b, w))
                        .collect::<Vec<_>>()
                        .join(",");
                    if variant == Variant::D {
                        format!(" [{cells}]")
                    } else {
                        format!(" {{{cells}}}")
                    }
                })
                .collect();
            let suffix = if variant == Variant::D {
                format!("[{ncols}][{}]", x.len())
            } else {
                format!("[{}][{ncols}]", x.len())
            };
            let (o, c) = if variant == Variant::D {
                ("[\n", "\n]")
            } else {
                ("{\n", "\n}")
            };
            (suffix, format!("{o}{}{c}", rows.join(",\n")))
        }
        Sexp::List(x) | Sexp::Vector(x) => {
            let cells = x.iter().map(|w| c_fmt(b, w)).collect::<Vec<_>>().join(",");
            (
                format!("[{}]", x.len()),
                if variant == Variant::D {
                    format!("[{cells}]")
                } else {
                    format!("{{{cells}}}")
                },
            )
        }
        v => (String::new(), c_fmt(b, v)),
    };
    if variant == Variant::D {
        format!("{ty}{suffix} {var} = {data};")
    } else {
        format!("{ty} {var}{suffix} = {data};")
    }
}

fn c_expand(env: &mut Env, body: &str, p: &[(String, Sexp)], variant: Variant) -> R<String> {
    let vars = params::vars(p);
    let colnames = items(get(p, ":colname-names").unwrap_or(&Sexp::Nil))
        .unwrap_or(&[])
        .to_vec();
    let main = get_str(p, ":main").as_deref() != Some("no");
    let strs = |key: &str| -> R<Vec<String>> {
        Ok(match get(p, key) {
            None => vec![],
            Some(v) => match value::read(&princ(v), false).unwrap_or(v.clone()) {
                Sexp::Str(s) => s.split_whitespace().map(str::to_owned).collect(),
                v if value::listp(&v) => items(&v).unwrap_or(&[]).iter().map(princ).collect(),
                v => vec![princ(&v)],
            },
        })
    };
    let mut out = vec![];
    if variant == Variant::D {
        let mut imports = match get(p, ":imports") {
            Some(v) => princ(v).split_whitespace().map(str::to_owned).collect(),
            None => env
                .entry("imports")
                .map(|s| s.split_whitespace().map(str::to_owned).collect())
                .unwrap_or_else(Vec::new),
        };
        imports.extend(["std.stdio".to_owned(), "std.conv".to_owned()]);
        out.push("module mmm;".to_owned());
        out.push(
            imports
                .iter()
                .map(|i| format!("import {i};"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    } else {
        let includes = strs(":includes")?;
        out.push(
            includes
                .iter()
                .map(|i| {
                    if i.starts_with('<') {
                        format!("#include {i}")
                    } else {
                        format!("#include \"{i}\"")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let defines: Vec<String> = match get(p, ":defines") {
            Some(Sexp::Str(s)) => s
                .split_whitespace()
                .collect::<Vec<_>>()
                .chunks(2)
                .map(|c| c.join(" "))
                .collect(),
            _ => strs(":defines")?,
        };
        out.push(
            defines
                .iter()
                .map(|d| format!("#define {d}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        out.push(
            strs(":namespaces")?
                .iter()
                .map(|n| format!("using namespace {n};"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    out.push(
        vars.iter()
            .map(|(n, v)| c_var(n, v, variant))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    out.push(
        vars.iter()
            .filter_map(|(n, v)| {
                let x = items(v).filter(|_| value::listp(v) && !v.is_nil())?;
                Some(
                    match x.first().and_then(|f| items(f).filter(|_| value::listp(f))) {
                        Some(first) => format!(
                            "const int {n}_rows = {};\nconst int {n}_cols = {};",
                            x.len(),
                            first.len()
                        ),
                        None => format!("const int {n}_cols = {};", x.len()),
                    },
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
    );
    if !colnames.is_empty() {
        out.push(if variant == Variant::D {
            "int get_column_num (string[] header, string column)\n{\n  foreach (c, h; header)\n    if (h==column)\n      return to!int(c);\n  return -1;\n}\n".into()
        } else {
            "\n#ifndef _STRING_H\n#include <string.h>\n#endif\nint get_column_num (int nbcols, const char** header, const char* column)\n{\n  int c;\n  for (c=0; c<nbcols; c++)\n    if (strcmp(header[c],column)==0)\n      return c;\n  return -1;\n}\n".into()
        });
    }
    out.push(
        colnames
            .iter()
            .filter_map(value::car_cdr)
            .map(|(t, headers)| {
                let t = princ(&t);
                let hs: Vec<String> = items(&headers).unwrap_or(&[]).iter().map(princ).collect();
                let tbl = vars.iter().find(|(n, _)| *n == t).map(|(_, v)| v.clone()).unwrap_or(Sexp::Nil);
                let typename = match base_type(&tbl) {
                    Some(Base::Int) => "int",
                    Some(Base::Float) => "double",
                    _ => if variant == Variant::D { "string" } else { "const char*" },
                };
                let quoted = hs.iter().map(|h| format!("\"{h}\"")).collect::<Vec<_>>().join(",");
                if variant == Variant::D {
                    format!("string[{}] {t}_header = [{quoted}];\n{typename} {t}_h (size_t row, string col) {{ return {t}[row][get_column_num({t}_header,col)]; }}", hs.len())
                } else {
                    format!("const char* {t}_header[{}] = {{{quoted}}};\n{typename} {t}_h (int row, const char* col) {{ return {t}[row][get_column_num({},{t}_header,col)]; }}", hs.len(), hs.len())
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let mut b = String::new();
    if let Some(pro) = get_str(p, ":prologue") {
        b.push_str(&pro);
        b.push('\n');
    }
    b.push_str(body);
    if let Some(epi) = get_str(p, ":epilogue") {
        b.push('\n');
        b.push_str(&epi);
        b.push('\n');
    }
    let has_main = regex::Regex::new(r"(?m)^[ \t]*[intvod]+[ \t\n\r]*main[ \t]*\(.*\)")
        .unwrap()
        .is_match(&b);
    out.push(if main && !has_main {
        format!("int main() {{\n{b}\nreturn 0;\n}}\n")
    } else {
        b
    });
    Ok(out.join("\n") + "\n")
}

fn c(env: &mut Env, body: &str, p: &Params, variant: Variant) -> R<Option<Sexp>> {
    let src = env.temp_file(
        "C-src-",
        match variant {
            Variant::C => ".c",
            Variant::Cpp => ".cpp",
            Variant::D => ".d",
        },
    );
    let bin = env.temp_file("C-bin-", "");
    let cmdline = get_str(p, ":cmdline")
        .map(|c| format!(" {c}"))
        .unwrap_or_default();
    let flags = list_or_str(get(p, ":flags"));
    let libs = match get(p, ":libs") {
        Some(v) => list_or_str(Some(&value::read(&princ(v), false).unwrap_or(v.clone()))),
        None => env
            .entry("libs")
            .map(|s| list_or_str(Some(&value::read(&s, false).unwrap_or(str(s)))))
            .unwrap_or_default(),
    };
    let fb = c_expand(env, body, p, variant)?;
    write(&src, &fb)?;
    let results = if variant == Variant::D {
        env.eval(
            &format!(
                "{} {flags} {}{cmdline}",
                opt("org-babel-D-compiler", "rdmd"),
                q(&src)
            ),
            "",
        )?
    } else {
        let compiler = if variant == Variant::C {
            opt("org-babel-C-compiler", "gcc")
        } else {
            opt("org-babel-C++-compiler", "g++")
        };
        env.eval(
            &format!("{compiler} -o {} {flags} {} {libs}", q(&bin), q(&src)),
            "",
        )?;
        env.eval(&format!("{}{cmdline}", q(&bin)), "")?
    };
    let results = value::remove_indentation(&results);
    let r = scalar_or_table(results, p, Sep::Auto)?;
    Ok(r.map(|r| reassemble(r, p)))
}

fn list_or_str(v: Option<&Sexp>) -> String {
    match v {
        None => String::new(),
        Some(v) if value::listp(v) => items(v)
            .unwrap_or(&[])
            .iter()
            .map(princ)
            .collect::<Vec<_>>()
            .join(" "),
        Some(v) => princ(v),
    }
}

fn fortran_var(n: &str, v: &Sexp) -> R<String> {
    let v = match v {
        Sexp::Sym(s) if s.chars().count() == 1 => Sexp::Int(s.chars().next().unwrap() as i64),
        Sexp::Sym(s) => str(s.clone()),
        v => v.clone(),
    };
    fn tl(v: &Sexp) -> String {
        match items(v).filter(|_| value::listp(v)) {
            Some(x) => format!("(/{}/)", x.iter().map(tl).collect::<Vec<_>>().join(", ")),
            None => prin1(v),
        }
    }
    Ok(match &v {
        Sexp::Int(i) => format!("integer, parameter  ::  {n} = {i}\n"),
        Sexp::Float(_) => format!("real, parameter ::  {n} = {}\n", prin1(&v)),
        Sexp::Str(s) => format!(
            "character(len={}), parameter ::  {n} = '{s}'\n",
            s.chars().count()
        ),
        Sexp::List(x) if x.iter().all(value::listp) => format!(
            "real, parameter :: {n}({},{}) = transpose( reshape( {} , (/ {}, {} /) ) )\n",
            x.len(),
            items(&x[0]).map_or(0, <[Sexp]>::len),
            tl(&v),
            items(&x[0]).map_or(0, <[Sexp]>::len),
            x.len()
        ),
        Sexp::List(x) => format!("real, parameter :: {n}({}) = {}\n", x.len(), tl(&v)),
        _ => {
            return err(format!(
                "The type of parameter {n} is not supported by ob-fortran"
            ));
        }
    })
}

fn fortran_expand(env: &mut Env, body: &str, p: &[(String, Sexp)]) -> R<String> {
    let vars = params::vars(p);
    let main = get_str(p, ":main").as_deref() != Some("no");
    let includes = get(p, ":includes").cloned().or_else(|| {
        env.entry("includes")
            .map(|s| value::read(&s, false).unwrap_or(str(s)))
    });
    let defines = get(p, ":defines").cloned().or_else(|| {
        env.entry("defines")
            .map(|s| value::read(&s, false).unwrap_or(str(s)))
    });
    let mut out = vec![];
    let lst = |v: &Sexp| -> Vec<String> {
        if value::listp(v) {
            items(v).unwrap_or(&[]).iter().map(princ).collect()
        } else {
            vec![princ(v)]
        }
    };
    if let Some(i) = includes {
        out.push(
            lst(&i)
                .iter()
                .map(|x| format!("#include {x}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    if let Some(d) = defines {
        out.push(
            lst(&d)
                .iter()
                .map(|x| format!("#define {x}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    let pro = get_str(p, ":prologue")
        .map(|x| x + "\n")
        .unwrap_or_default();
    if main {
        let mut inner = String::new();
        for (n, v) in &vars {
            inner.push_str(&fortran_var(n, v)?);
            inner.push('\n');
        }
        inner.push_str(&pro);
        inner.push_str(body);
        inner.push_str(&pro);
        if regex::Regex::new(r"(?mi)^[ \t]*program\b")
            .unwrap()
            .is_match(&inner)
        {
            if !vars.is_empty() {
                return err("Cannot use :vars if `program' statement is present");
            }
            out.push(inner);
        } else {
            out.push(format!("program main\n{inner}\nend program main\n"));
        }
    } else {
        out.push(format!(
            "{pro}{body}{}",
            get_str(p, ":epilogue")
                .map(|e| format!("\n{e}\n"))
                .unwrap_or_default()
        ));
    }
    Ok(out.join("\n"))
}

fn fortran(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let src = env.temp_file("fortran-src-", ".F90");
    let bin = env.temp_file("fortran-bin-", "");
    let fb = fortran_expand(env, body, p)?;
    write(&src, &fb)?;
    env.eval(
        &format!(
            "{} -o {} {} {}",
            opt("org-babel-fortran-compiler", "gfortran"),
            q(&bin),
            list_or_str(get(p, ":flags")),
            q(&src)
        ),
        "",
    )?;
    let out = env.eval(
        &format!(
            "{}{}",
            q(&bin),
            get_str(p, ":cmdline")
                .map(|c| format!(" {c}"))
                .unwrap_or_default()
        ),
        "",
    )?;
    let results = value::trim(&value::remove_indentation(&out)).to_owned();
    let r = match result_cond(&rp(p)) {
        None => None,
        Some(true) => Some(value::read(&results, false)?),
        Some(false) => Some(value::import_table(&results, Sep::Auto)?),
    };
    Ok(r.map(|r| reassemble(r, p)))
}

// java

fn java_find_classname(body: &str) -> String {
    let pkg = regex::Regex::new(r"(?m)^\s*package\s+([\w.]+)\s*;$")
        .unwrap()
        .captures(body)
        .map(|c| c[1].to_owned());
    let class = regex::Regex::new(r"(?m)^\s*(?:public\s+)?class\s+(\w+)\s*\{")
        .unwrap()
        .captures(body)
        .map(|c| c[1].to_owned());
    match (pkg, class) {
        (Some(p), Some(c)) => format!("{p}.{c}"),
        (None, Some(c)) => c,
        (Some(p), None) => format!("{p}.Main"),
        _ => "Main".into(),
    }
}

fn java_type(v: &Sexp) -> (Option<Base>, String) {
    let b = base_type(v);
    let s = match b {
        Some(Base::Int) => "Integer",
        Some(Base::Float) => "Double",
        _ => "String",
    };
    let t = match v {
        Sexp::List(x) if x.first().is_some_and(|f| matches!(f, Sexp::List(_))) => {
            format!("List<List<{s}>>")
        }
        Sexp::List(_) | Sexp::Vector(_) => format!("List<{s}>"),
        _ => s.to_owned(),
    };
    (b, t)
}

fn java_var(v: &Sexp, b: Option<Base>) -> R<String> {
    Ok(match v {
        Sexp::List(x) | Sexp::Vector(x) => format!(
            "Arrays.asList({})",
            x.iter()
                .map(|e| java_var(e, b))
                .collect::<R<Vec<_>>>()?
                .join(", ")
        ),
        v if is_hline(v) => opt("org-babel-java-hline-to", "null"),
        _ if b == Some(Base::Int) => princ(v),
        Sexp::Float(f) if b == Some(Base::Float) => format!("{f:.6}"),
        Sexp::Int(i) if b == Some(Base::Float) => format!("{:.6}", *i as f64),
        Sexp::Str(s) if regex::Regex::new(r".\n+.").unwrap().is_match(s) => {
            return err("Java does not support multiline string literals");
        }
        v => format!("\"{}\"", princ(v)),
    })
}

fn java_assignments(p: &[(String, Sexp)]) -> R<Vec<String>> {
    params::vars(p)
        .iter()
        .map(|(n, v)| {
            let (b, t) = java_type(v);
            Ok(format!("    static {t} {n} = {};", java_var(v, b)?))
        })
        .collect()
}

/// Move past every match of RE in S from `at`: the position after the
/// last match end + 1 (org-babel-java--move-past).
fn move_past(s: &str, re: &regex::Regex) -> usize {
    let mut pos = 0;
    while let Some(m) = re.find_at(s, pos) {
        pos = (m.end() + 1).min(s.len());
        if pos >= s.len() {
            break;
        }
    }
    pos
}

fn java_res() -> [regex::Regex; 4] {
    [
        regex::Regex::new(r"(?m)^\s*package\s+([\w.]+)\s*;$").unwrap(),
        regex::Regex::new(r"(?m)^\s*import(?:\s+static)?\s+([\w.*]+)\s*;$").unwrap(),
        regex::Regex::new(r"(?m)^\s*(?:public\s+)?class\s+(\w+)\s*\{").unwrap(),
        regex::Regex::new(r"(?m)^\s*public\s+static\s+void\s+main\s*\(\s*String[\w\[\] ]+\)\s*(?:throws[\w,. ]+)?\{").unwrap(),
    ]
}

fn java_expand(body: &str, p: &[(String, Sexp)]) -> R<String> {
    let [pkg_re, imp_re, class_re, main_re] = java_res();
    let any_method = regex::Regex::new(r"(?m)^\s*(?:\w+\s+)?(?:static\s+)?[\w\[\]]+\s+\w+\s*\([\w\[\], ]*\)\s*(?:throws[\w,. ]+)?\{").unwrap();
    let full = get_str(p, ":classname").unwrap_or_else(|| java_find_classname(body));
    let classname = full.rsplit('.').next().unwrap_or(&full).to_owned();
    let package = full
        .contains('.')
        .then(|| full.rsplit_once('.').map(|(a, _)| a.to_owned()))
        .flatten();
    let vars = java_assignments(p)?;
    let imports: Vec<String> = get(p, ":imports")
        .map(|v| {
            princ(&value::read(&princ(v), false).unwrap_or(v.clone()))
                .split(' ')
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let mut s = String::new();
    if let Some(pro) = get_str(p, ":prologue") {
        s.push_str(&pro);
        s.push('\n');
    }
    s.push_str(body);
    if let Some(epi) = get_str(p, ":epilogue") {
        s.push('\n');
        s.push_str(&epi);
    }
    let indent = |t: &str| {
        t.split('\n')
            .map(|l| {
                if l.is_empty() {
                    l.to_owned()
                } else {
                    format!("    {l}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    if !main_re.is_match(&s) && !any_method.is_match(&s) {
        let at = move_past(&s, &pkg_re).max(move_past(&s, &imp_re));
        s = format!(
            "{}public static void main(String[] args) {{\n{}\n}}",
            &s[..at],
            indent(&s[at..])
        );
    }
    if !class_re.is_match(&s) {
        let at = move_past(&s, &pkg_re).max(move_past(&s, &imp_re));
        let lead = if package.is_some() || !imports.is_empty() {
            "\n"
        } else {
            ""
        };
        s = format!(
            "{}{lead}public class {classname} {{\n{}\n}}",
            &s[..at],
            indent(&s[at..])
        );
    }
    if !vars.is_empty() {
        let at = move_past(&s, &class_re);
        s = format!("{}{}\n{}", &s[..at], vars.join("\n"), &s[at..]);
    }
    if !imports.is_empty() {
        let at = move_past(&s, &pkg_re);
        s = format!(
            "{}{}\n{}",
            &s[..at],
            imports
                .iter()
                .map(|i| format!("import {i};"))
                .collect::<Vec<_>>()
                .join("\n"),
            &s[at..]
        );
    }
    if let Some(pk) = package
        && !pkg_re.is_match(&s)
    {
        s = format!("package {pk};\n{s}");
    }
    Ok(s.trim_start_matches('\n').to_owned())
}

const JAVA_RESULT_WRAPPER: &str = "\n    public static String __toString(Object val) {\n        if (val instanceof String) {\n            return \"\\\"\" + val + \"\\\"\";\n        } else if (val == null) {\n            return \"null\";\n        } else if (val.getClass().isArray()) {\n            StringBuffer sb = new StringBuffer();\n            Object[] vals = (Object[])val;\n            sb.append(\"[\");\n            for (int ii=0; ii<vals.length; ii++) {\n                sb.append(__toString(vals[ii]));\n                if (ii<vals.length-1)\n                    sb.append(\",\");\n            }\n            sb.append(\"]\");\n            return sb.toString();\n        } else if (val instanceof List) {\n            StringBuffer sb = new StringBuffer();\n            List vals = (List)val;\n            sb.append(\"[\");\n            for (int ii=0; ii<vals.size(); ii++) {\n                sb.append(__toString(vals.get(ii)));\n                if (ii<vals.size()-1)\n                    sb.append(\",\");\n            }\n            sb.append(\"]\");\n            return sb.toString();\n        } else {\n            return String.valueOf(val);\n        }\n    }\n\n    public static void main(String[] args) throws IOException {\n        BufferedWriter output = new BufferedWriter(new FileWriter(\"%f\"));\n        output.write(__toString(_main(args)));\n        output.close();\n    }";

fn java(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let [pkg_re, _, class_re, main_re] = java_res();
    let javac = get_str(p, ":javac").unwrap_or_else(|| opt("org-babel-java-compiler", "javac"));
    let java = get_str(p, ":java").unwrap_or_else(|| opt("org-babel-java-command", "java"));
    let from_temp = !has(p, ":dir") || get(p, ":dir").is_some_and(Sexp::is_nil);
    let full_class = get_str(p, ":classname").unwrap_or_else(|| java_find_classname(body));
    let classname = full_class
        .rsplit('.')
        .next()
        .unwrap_or(&full_class)
        .to_owned();
    let package = full_class.rsplit_once('.').map(|(a, _)| a.to_owned());
    let basedir = if from_temp {
        core::temp_dir()
    } else {
        env.dir.clone()
    };
    let pkgdir = match (&package, from_temp) {
        (Some(pk), false) => basedir.join(pk.replace('.', "/")),
        _ => basedir.clone(),
    };
    std::fs::create_dir_all(&pkgdir).map_err(|e| e.to_string())?;
    let src = pkgdir.join(format!("{classname}.java"));
    let value_mode = result_type(p) == "value";
    let result_file = value_mode.then(|| env.temp_file("java-", ""));
    let mut s = java_expand(body, p)?;
    if from_temp && let Some(m) = pkg_re.find(&s) {
        s.replace_range(m.range(), "");
    }
    if !main_re.is_match(&s) {
        let at = move_past(&s, &class_re);
        s.insert_str(at, "\n    public static void main(String[] args) {\n        System.out.print(\"success\");\n    }\n\n");
    }
    if let Some(rf) = &result_file {
        let at = move_past(&s, &class_re);
        s.insert_str(
            at,
            &JAVA_RESULT_WRAPPER.replace("%f", &rf.display().to_string()),
        );
        if let Some(i) = s[at..].find("public static void main(").map(|i| i + at) {
            let i2 = s[i + 1..]
                .find("public static void main(")
                .map(|j| j + i + 1);
            // Rename the block's own main (the one after the wrapper's).
            if let Some(j) = i2 {
                s.replace_range(j..j + 24, "public static Object _main(");
            }
        }
    }
    for (pkg, cls) in [
        ("java.util", "List"),
        ("java.util", "Arrays"),
        ("java.io", "BufferedWriter"),
        ("java.io", "FileWriter"),
        ("java.io", "IOException"),
    ] {
        if s.contains(cls)
            && !regex::Regex::new(&format!(r"(?m)^import .*{pkg}.*(?:\*|{cls});"))
                .unwrap()
                .is_match(&s)
        {
            let at = move_past(&s, &pkg_re);
            s.insert_str(at, &format!("import {pkg}.{cls};\n"));
        }
    }
    write(&src, &s)?;
    let cmd = format!(
        "{javac} {} {} && {java} -cp {} {} {} {}",
        get_str(p, ":cmpflag").unwrap_or_default(),
        q(&src),
        q(&basedir),
        get_str(p, ":cmdline").unwrap_or_default(),
        if from_temp { classname } else { full_class },
        get_str(p, ":cmdargs").unwrap_or_default()
    );
    let raw = match &result_file {
        None => env.eval(&cmd, "")?,
        Some(f) => {
            env.eval(&cmd, "")?;
            read_file(f)
        }
    };
    let r = match result_cond(&rp(p)) {
        None => None,
        Some(true) => Some(str(raw)),
        Some(false) => {
            let res = value::script_escape(&raw, false);
            Some(match items(&res) {
                Some(x) if !res.is_nil() => list(
                    x.iter()
                        .map(|e| {
                            if matches!(e, Sexp::Sym(s) if s == "null") {
                                Sexp::Sym(opt("org-babel-java-null-to", "hline"))
                            } else {
                                e.clone()
                            }
                        })
                        .collect(),
                ),
                _ => res,
            })
        }
    };
    Ok(r.map(|r| reassemble(r, p)))
}

fn groovy(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    if session_name(p).is_some() {
        return err("Sessions are not (yet) supported for Groovy");
    }
    let fb = expand_generic(body, p, &[]);
    let src = env.temp_file("groovy_", "");
    let cmd = opt("org-babel-groovy-command", "groovy");
    let r = if result_type(p) == "output" {
        write(&src, &fb)?;
        str(env.eval(&format!("{cmd} {}", q(&src)), "")?)
    } else {
        write(
            &src,
            &format!(
                "class Runner extends Script {{\n    def out = new PrintWriter(new ByteArrayOutputStream())\n    def run() {{ {fb} }}\n}}\n\nprintln(new Runner().run())\n"
            ),
        )?;
        let raw = env.eval(&format!("{cmd} {}", q(&src)), "")?;
        match result_cond(&rp(p)) {
            None => return Ok(None),
            Some(true) => str(raw),
            Some(false) => value::script_escape(&raw, false),
        }
    };
    Ok(Some(reassemble(r, p)))
}

fn haskell_var(v: &Sexp) -> String {
    match items(v).filter(|_| value::listp(v)) {
        Some(x) => format!(
            "[{}]",
            x.iter().map(haskell_var).collect::<Vec<_>>().join(", ")
        ),
        None => prin1(v),
    }
}

fn haskell(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    if get_str(p, ":compile").as_deref() == Some("yes") {
        let src = env.temp_file("Haskell-src-", ".hs");
        let bin = env.temp_file("Haskell-bin-", "");
        write(&src, body)?;
        let libs = get(p, ":libs")
            .map(|v| list_or_str(Some(v)))
            .or_else(|| env.entry("libs"))
            .unwrap_or_default();
        env.eval(
            &format!(
                "{} -o {} {} {} {libs}",
                opt("org-babel-haskell-compiler", "ghc"),
                q(&bin),
                list_or_str(get(p, ":flags")),
                q(&src)
            ),
            "",
        )?;
        let out = env.eval(
            &format!(
                "{}{}",
                q(&bin),
                get_str(p, ":cmdline")
                    .map(|c| format!(" {c}"))
                    .unwrap_or_default()
            ),
            "",
        )?;
        let results = value::trim(&value::remove_indentation(&out)).to_owned();
        let r = match result_cond(&rp(p)) {
            None => None,
            Some(true) => Some(value::read(&results, true)?),
            Some(false) => Some(value::import_table(&results, Sep::Auto)?),
        };
        return Ok(r.map(|r| reassemble(r, p)));
    }
    let name = match session_name(p) {
        Some(n) if !n.is_empty() => n,
        Some(_) => "*ob-haskell*".into(),
        None => format!(" *ob-haskell-tmp-{}*", core::temp_file("", "").display()),
    };
    let kind = session::Kind::Haskell;
    let fb = full(env, "haskell", body, p)?;
    let raw = if result_type(p) == "output" {
        session::eval(&kind, &name, &env.dir, value::trim(&fb))?
    } else {
        session::eval(
            &kind,
            &name,
            &env.dir,
            &format!(
                "__LAST_VALUE_IMPROBABLE_NAME__=()::()\n{}\n__LAST_VALUE_IMPROBABLE_NAME__=it",
                value::trim(&fb)
            ),
        )?;
        session::eval(&kind, &name, &env.dir, "__LAST_VALUE_IMPROBABLE_NAME__")?
    };
    if session_name(p).is_none() {
        session::kill(&kind, &name);
    }
    let result = value::trim(&raw).trim_matches('"').to_owned();
    let r = match result_cond(&rp(p)) {
        None => None,
        Some(true) => Some(str(result)),
        Some(false) => Some(value::script_escape(&result, false)),
    };
    Ok(r.map(|r| reassemble(r, p)))
}

fn ocaml_var(v: &Sexp) -> String {
    match items(v).filter(|_| value::listp(v)) {
        Some(x) => format!(
            "[|{}|]",
            x.iter().map(ocaml_var).collect::<Vec<_>>().join("; ")
        ),
        None => prin1(v),
    }
}

fn ocaml(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let fb = full(env, "ocaml", body, p)?;
    let name = session_name(p)
        .filter(|n| !n.is_empty() && n != "default")
        .unwrap_or_else(|| "*ocaml-toplevel*".into());
    let raw = session::eval(
        &session::Kind::Ocaml(opt("org-babel-ocaml-command", "ocaml")),
        &name,
        &env.dir,
        &format!("{};;", value::chomp(&fb, value::WS)),
    )?;
    let raw = value::trim(&raw).to_owned();
    let rp = rp(p);
    let re = regex::Regex::new(r"(?s)^((?:.*\n)*)[^:\n]+ : ([^=\n]+) =\s+(.+)$").unwrap();
    let (output, ty, val) = match re.captures(&raw) {
        Some(c) => (
            c[1].to_owned(),
            Some(c[2].trim().to_owned()),
            Some(c[3].to_owned()),
        ),
        None => (String::new(), None, None),
    };
    let r = match result_cond(&rp) {
        None => None,
        Some(true) => Some(str(
            if rp.iter().any(|r| r == "output") && !rp.iter().any(|r| r == "verbatim") {
                output
            } else {
                raw
            },
        )),
        Some(false) => Some(match (val, ty) {
            (Some(v), Some(t)) => match t.as_str() {
                "string" => value::read(&v, false)?,
                "int" | "float" => value::read_number(&v).unwrap_or(Sexp::Int(0)),
                t if t.contains("list") => value::script_escape(&v.replace(';', ","), false),
                t if t.contains("array") => value::script_escape(
                    &v.replace("; ", ",").replace("|]", "]").replace("[|", "["),
                    false,
                ),
                _ => str(v),
            },
            _ => str(raw),
        }),
    };
    Ok(r.map(|r| reassemble(r, p)))
}

fn clojure_expand(body: &str, p: &[(String, Sexp)]) -> String {
    let vars = params::vars(p);
    let ns = get_str(p, ":ns");
    let b = if vars.is_empty() {
        value::trim(body).to_owned()
    } else {
        format!(
            "(let [{}]\n{body}\n)",
            vars.iter()
                .map(|(n, v)| format!("{n} '{}", prin1(v)))
                .collect::<Vec<_>>()
                .join("\n      ")
        )
    };
    value::trim(&format!(
        "{}{b}",
        ns.map(|n| format!("(ns {n})\n")).unwrap_or_default()
    ))
    .to_owned()
}

fn clojure(env: &mut Env, body: &str, p: &Params, cljs: bool) -> R<Option<Sexp>> {
    let backend = get_str(p, ":backend").unwrap_or_else(|| {
        if cljs {
            opt(
                "org-babel-clojurescript-backend",
                if which("nbb") || which("npx") {
                    "nbb"
                } else {
                    "nil"
                },
            )
        } else {
            opt(
                "org-babel-clojure-backend",
                if which("bb") {
                    "babashka"
                } else if which("clojure") {
                    "clojure-cli"
                } else {
                    "nil"
                },
            )
        }
    });
    let cljs = cljs || backend == "nbb";
    let rp = rp(p);
    let expanded = clojure_expand(body, p);
    let prepared = if rp.iter().any(|r| r == "output") {
        expanded
    } else {
        format!(
            "{}{}{expanded}\n))",
            if rp.iter().any(|r| r == "code" || r == "pp") {
                format!(
                    "{} (pprint",
                    if cljs {
                        "(require '[cljs.pprint :refer [pprint]])"
                    } else {
                        "(require '[clojure.pprint :refer [pprint]])"
                    }
                )
            } else {
                "(prn ".into()
            },
            if cljs {
                "(binding [cljs.core/*print-fn* (constantly nil)]"
            } else {
                "(binding [*out* (java.io.StringWriter.)]\n"
            }
        )
    };
    let cmd = match backend.as_str() {
        "babashka" => opt("ob-clojure-babashka-command", "bb"),
        "clojure-cli" => opt("ob-clojure-cli-command", "clojure -M"),
        "nbb" => opt(
            "ob-clojure-nbb-command",
            if which("nbb") { "nbb" } else { "npx nbb" },
        ),
        "nil" => {
            return err(
                "You need to customize `org-babel-clojure-backend'\nor set the `:backend' header argument",
            );
        }
        b => {
            return err(format!(
                "Invalid backend {b} (Fred runs Clojure with babashka, clojure-cli or nbb)"
            ));
        }
    };
    let script = env.temp_file("clojure-cmd-script-", ".clj");
    write(&script, &prepared)?;
    let result = env.eval(&format!("{cmd} {}", q(&script)), "")?;
    Ok(match result_cond(&rp) {
        None => None,
        Some(true) => Some(str(result)),
        Some(false) => Some(value::script_escape(&result, false)),
    })
}

fn scheme(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let imp = get_str(p, ":scheme")
        .or_else(|| sexp::option("geiser-scheme-implementation").and_then(|v| as_string(&v)))
        .or_else(|| sexp::option("geiser-default-implementation").and_then(|v| as_string(&v)))
        .or_else(|| {
            sexp::option("geiser-active-implementations")
                .and_then(|v| items(&v).and_then(|x| x.first().and_then(as_string)))
        })
        .unwrap_or_else(|| "guile".into());
    let fb = expand(env, "scheme", body, p)?.unwrap_or_default();
    let output = result_type(p) == "output";
    let rp = rp(p);
    let src = env.temp_file("scheme-", ".scm");
    let code = if output {
        fb
    } else {
        format!("(write (begin {fb}\n))")
    };
    write(&src, &code)?;
    let cmd = match imp.as_str() {
        "guile" => format!("guile --no-auto-compile -s {}", q(&src)),
        "racket" => format!("racket {}", q(&src)),
        "chicken" => format!("csi -s {}", q(&src)),
        "chez" => format!("scheme --script {}", q(&src)),
        "mit" => format!("mit-scheme --quiet < {}", q(&src)),
        "chibi" => format!("chibi-scheme {}", q(&src)),
        "gambit" => format!("gsi {}", q(&src)),
        i => format!("{i} {}", q(&src)),
    };
    let result = env.eval(&cmd, "")?;
    let table = {
        let res = value::script_escape(&result, false);
        match items(&res) {
            Some(x) if value::listp(&res) => list(
                x.iter()
                    .map(|e| {
                        if e.is_nil() || matches!(e, Sexp::Sym(s) if s == "null") {
                            Sexp::Sym(opt("org-babel-scheme-null-to", "hline"))
                        } else {
                            e.clone()
                        }
                    })
                    .collect(),
            ),
            _ => res,
        }
    };
    Ok(match result_cond(&rp) {
        None => None,
        Some(true) => Some(str(result)),
        Some(false) => Some(reassemble(table, p)),
    })
}

fn lisp_expand(body: &str, p: &[(String, Sexp)]) -> String {
    let vars = params::vars(p);
    let rp = rp(p);
    let b = if vars.is_empty() {
        value::trim(body).to_owned()
    } else {
        format!(
            "(cl:let ({})\n{}{body}{})",
            vars.iter()
                .map(|(n, v)| format!("({n} (cl:quote {}))", prin1(v)))
                .collect::<Vec<_>>()
                .join("\n      "),
            get_str(p, ":prologue")
                .map(|x| x + "\n")
                .unwrap_or_default(),
            get_str(p, ":epilogue")
                .map(|x| format!("\n{x}\n"))
                .unwrap_or_default()
        )
    };
    if rp.iter().any(|r| r == "code" || r == "pp") {
        format!("(cl:pprint {b})")
    } else {
        b
    }
}

fn lisp(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let code = lisp_expand(body, p);
    let dir = get_str(p, ":dir")
        .map(|d| env.dir.join(d))
        .unwrap_or_else(|| env.dir.clone());
    let wrapped = format!(
        "(let* ((out (make-string-output-stream)) (vals (let ((*standard-output* out) (*default-pathname-defaults* #P{})) (multiple-value-list (progn {code}\n))))) (format t \"~a~%org-babel-lisp-sep~%~{{~S~^ ~}}\" (get-output-stream-string out) vals))",
        prin1(&str(format!("{}/", dir.display())))
    );
    let src = env.temp_file("lisp-", ".lisp");
    write(&src, &wrapped)?;
    let raw = env.eval(
        &format!(
            "{} --script {}",
            opt("inferior-lisp-program", "sbcl"),
            q(&src)
        ),
        "",
    )?;
    let (out, val) = raw.split_once("org-babel-lisp-sep\n").unwrap_or((&raw, ""));
    let result = if rp(p).iter().any(|r| r == "output") {
        out.trim_end_matches('\n').to_owned()
    } else {
        val.to_owned()
    };
    let r = match result_cond(&rp(p)) {
        None => None,
        Some(true) => Some(str(result.trim_matches('"'))),
        Some(false) => Some(sexp::read(&result.replace("#(", "(")).unwrap_or(str(result))),
    };
    Ok(r.map(|r| reassemble(r, p)))
}

fn forth(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    if get_str(p, ":session").as_deref() == Some("none") {
        return err("Non-session evaluation not supported for Forth code blocks");
    }
    let lines: Vec<String> = value::trim(&expand_generic(body, p, &[]))
        .split('\n')
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    let name = session_name(p).unwrap_or_else(|| "*forth*".into());
    let mut results = vec![];
    for l in lines {
        let out = session::eval(
            &session::Kind::Plain(
                vec![opt("forth-executable", "gforth")],
                ".\" org_babel_forth_eoe\" cr".into(),
                "org_babel_forth_eoe".into(),
            ),
            &name,
            &env.dir,
            &l,
        )?;
        results.push(
            value::trim(
                out.trim_end()
                    .trim_end_matches("ok")
                    .trim_end_matches("compiled"),
            )
            .to_owned(),
        );
    }
    Ok(if rp(p).iter().any(|r| r == "output") {
        Some(str(results.join("\n")))
    } else {
        results.last().cloned().map(str)
    })
}

fn awk_var(v: &Sexp) -> String {
    match items(v) {
        Some(x) if x.first().is_some_and(value::listp) => {
            value::orgtbl_to_generic(v, "\t", &echo, None)
        }
        Some(x) if !v.is_nil() => x.iter().map(echo).collect::<Vec<_>>().join("\n"),
        _ => echo(v),
    }
}

fn awk(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let code = env.temp_file("awk-", "");
    write(&code, &wrap_pro_epi(body, p))?;
    let stdin = match get_str(p, ":stdin") {
        Some(s) => Some(awk_var(&env.resolve(&s)?)),
        None => None,
    };
    let mut cmd = vec![
        opt("org-babel-awk-command", "awk"),
        get_str(p, ":cmd-line").unwrap_or_default(),
        "-f".into(),
        q(&code),
    ];
    for (n, v) in params::vars(p) {
        cmd.push(format!("-v {n}='{}'", awk_var(&v)));
    }
    if let Some(f) = get_str(p, ":in-file") {
        cmd.push(q(&env.dir.join(f)));
    }
    let cmd = cmd.join(" ");
    let results = match stdin {
        Some(s) => {
            let (o, e, _) = core::run_shell(&cmd, &s, &env.dir)?;
            o + &e
        }
        None => env.eval(&cmd, "")?,
    };
    let r = scalar_or_table(results, p, Sep::Auto)?;
    Ok(r.map(|r| reassemble(r, p)))
}

fn sed(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let code = env.temp_file("sed-", "");
    write(&code, body)?;
    let stdin = match get_str(p, ":stdin") {
        Some(s) => Some(echo(&env.resolve(&s)?)),
        None => None,
    };
    let mut cmd = vec![
        opt("org-babel-sed-command", "sed"),
        format!("-f \"{}\"", code.display()),
    ];
    cmd.extend(get_str(p, ":cmd-line"));
    cmd.extend(get_str(p, ":in-file"));
    let cmd = cmd.join(" ");
    let results = match stdin {
        Some(s) => {
            let (o, e, _) = core::run_shell(&cmd, &s, &env.dir)?;
            o + &e
        }
        None => env.eval(&cmd, "")?,
    };
    let r = scalar_or_table(results, p, Sep::Auto)?;
    Ok(r.map(|r| reassemble(r, p)))
}

fn calc(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    // Calc is Emacs's: run ob-calc itself.
    let r = emacs_babel(env, "ob-calc", "org-babel-execute:calc", body, p)?;
    Ok(Some(r))
}

fn sass(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let file = get_str(p, ":file");
    let out = file
        .as_ref()
        .map(|f| env.dir.join(f))
        .unwrap_or_else(|| env.temp_file("sass-out-", ""));
    let input = env.temp_file("sass-in-", "");
    write(&input, &expand_generic(body, p, &[]))?;
    env.eval(
        &format!(
            "sass {} {} {}",
            get_str(p, ":cmdline").unwrap_or_default(),
            q(&input),
            q(&out)
        ),
        "",
    )?;
    Ok(if file.is_some() {
        None
    } else {
        Some(str(read_file(&out)))
    })
}

fn processing_var(n: &str, v: &Sexp) -> String {
    let v = match v {
        Sexp::Sym(s) => str(s.clone()),
        v => v.clone(),
    };
    let ty = |data: &Sexp| -> &str {
        fn find(d: &Sexp, t: &mut &'static str) -> bool {
            for e in items(d).unwrap_or(&[]) {
                if value::listp(e) && !e.is_nil() {
                    if find(e, t) {
                        return true;
                    }
                } else if matches!(e, Sexp::Str(_)) {
                    *t = "String";
                    return true;
                } else if matches!(e, Sexp::Float(_)) {
                    *t = "float";
                }
            }
            false
        }
        let mut t = "int";
        find(data, &mut t);
        t
    };
    match &v {
        Sexp::Int(i) => format!("int {n}={i};"),
        Sexp::Float(_) => format!("float {n}={};", prin1(&v)),
        Sexp::Str(s) => format!("String {n}=\"{s}\";"),
        Sexp::List(x) if !x.first().is_some_and(value::listp) => {
            let t = ty(&v);
            let fmt = |e: &Sexp| {
                if t == "String" {
                    format!("\"{}\"", princ(e))
                } else {
                    princ(e)
                }
            };
            format!(
                "{t}[] {n}={{{}}};",
                x.iter().map(fmt).collect::<Vec<_>>().join(", ")
            )
        }
        Sexp::List(x) => {
            let t = ty(&v);
            let fmt = |e: &Sexp| {
                if t == "String" {
                    format!("\"{}\"", princ(e))
                } else {
                    princ(e)
                }
            };
            format!(
                "{t}[][] {n}={{{}}};",
                x.iter()
                    .map(|r| format!(
                        "{{{}}}",
                        items(r)
                            .unwrap_or(&[])
                            .iter()
                            .map(fmt)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        _ => String::new(),
    }
}

fn processing(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let code = full(env, "processing", body, p)?;
    let id = format!("ob-{}", super::sha1::hex(code.as_bytes()));
    Ok(Some(str(format!(
        "<script src=\"{}\"></script>\n <script type=\"text/processing\" data-processing-target=\"{id}\">\n{code}\n</script> <canvas id=\"{id}\"></canvas>",
        opt(
            "org-babel-processing-processing-js-filename",
            "processing.js"
        )
    ))))
}

fn screen(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let session = get_str(p, ":session").unwrap_or_else(|| "default".into());
    let screen = opt("org-babel-screen-location", "screen");
    let socket = |dir: &Path| -> R<Option<String>> {
        let (ls, _, _) = core::run_shell(&format!("{screen} -ls"), "", dir)?;
        Ok(ls
            .lines()
            .filter(|l| l.contains("(Attached)") || l.contains("(Detached)"))
            .find(|l| l.contains(&session))
            .and_then(|l| l.split_whitespace().next().map(str::to_owned)))
    };
    if socket(&env.dir)?.is_none() {
        // Fred has no terminal to open: start the session detached.
        let cmd = get_str(p, ":cmd").unwrap_or_else(|| "sh".into());
        core::run_shell(
            &format!("{screen} -dmS {} {}", core::shell_quote(&session), cmd),
            "",
            &env.dir,
        )?;
    }
    let Some(sock) = socket(&env.dir)? else {
        return err(format!("screen session {session} did not start"));
    };
    let tmp = env.temp_file("screen-", "");
    let text: String = format!("{}\n", expand_generic(body, p, &[]))
        .lines()
        .filter(|l| !l.chars().all(|c| c == ' ') || l.is_empty())
        .map(|l| format!("{l}\n"))
        .collect();
    write(&tmp, &text)?;
    core::run_shell(
        &format!(
            "{screen} -S {} -X eval 'msgwait 0' {} 'paste z'",
            core::shell_quote(&sock),
            core::shell_quote(&format!("readreg z {}", tmp.display()))
        ),
        "",
        &env.dir,
    )?;
    Ok(None)
}

fn org(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let rparams: Vec<String> = get_str(p, ":results")
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let unescaped: String = body
        .split('\n')
        .map(|l| l.strip_prefix(',').unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n");
    let b = expand(env, "org", &unescaped, p)?.unwrap_or_default();
    if let Some(backend) = ["latex", "html", "ascii"]
        .into_iter()
        .find(|b| rparams.iter().any(|r| r == b))
    {
        // Fred's exporter is a separate module; ask it by name.
        return match super::export_string(&b, backend) {
            Some(s) => Ok(Some(str(s))),
            None => err(format!("Export to {backend} is not available")),
        };
    }
    Ok(Some(str(b)))
}

fn csharp_expand(body: &str, p: &[(String, Sexp)]) -> String {
    let main = get_str(p, ":main").as_deref() != Some("no");
    let class = match get(p, ":class") {
        Some(v) if as_string(v).as_deref() == Some("no") => None,
        None | Some(Sexp::Nil) => Some("Program".to_owned()),
        Some(v) => as_string(v),
    };
    let mut s = String::new();
    if let Some(pro) = get_str(p, ":prologue") {
        s.push_str(&pro);
        s.push('\n');
    }
    s.push_str("namespace org.babel.autogen;\n");
    if let Some(u) = get(p, ":usings") {
        let us: Vec<String> = items(u)
            .map(|x| x.iter().map(princ).collect())
            .unwrap_or_else(|| vec![princ(u)]);
        s.push_str(&format!(
            "\n{}\n",
            us.iter()
                .map(|x| format!("using {x};"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if let Some(c) = &class {
        s.push_str(&format!("\nclass {c}\n{{\n"));
    }
    if main {
        s.push_str("static void Main(string[] args)\n{\n");
    }
    let vars: Vec<String> = params::vars(p)
        .iter()
        .map(|(n, v)| format!("var {n} = {};", prin1(v)))
        .collect();
    s.push_str(&vars.join("\n"));
    s.push('\n');
    s.push_str(body);
    if main {
        s.push_str("\n}");
    }
    if class.is_some() {
        s.push_str("\n}");
    }
    if let Some(epi) = get_str(p, ":epilogue") {
        s.push('\n');
        s.push_str(&epi);
    }
    s
}

fn csharp(env: &mut Env, body: &str, p: &Params) -> R<Option<Sexp>> {
    let dotnet = opt("org-babel-csharp-compiler", "dotnet");
    let (sdks, _, _) = core::run_shell(&format!("{dotnet} --list-sdks"), "", &env.dir)?;
    let versions: Vec<u32> = sdks
        .lines()
        .filter_map(|l| l.split('.').next()?.trim().parse().ok())
        .filter(|v| *v > 0)
        .collect();
    if versions.is_empty() {
        return err("Could not find a .NET SDK for compiling");
    }
    let base = core::temp_dir().join(format!(
        "obcs{}",
        core::temp_file("", "")
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("x")
    ));
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let project = base
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("obcs")
        .to_owned();
    let bin = base.join("bin");
    let framework = get_str(p, ":framework").unwrap_or_else(|| {
        opt(
            "org-babel-csharp-default-target-framework",
            &format!("net{}.0", versions.iter().max().unwrap()),
        )
    });
    write(&base.join("Program.cs"), &csharp_expand(body, p))?;
    let refs: String = get(p, ":references")
        .map(|r| {
            let mut proj = String::new();
            let mut asm = String::new();
            let mut pkg = String::new();
            for e in items(r).unwrap_or(std::slice::from_ref(r)) {
                let (name, ver) = match value::car_cdr(e).filter(|_| matches!(e, Sexp::Dotted(..))) {
                    Some((n, v)) => (princ(&n), Some(princ(&v))),
                    None => (princ(e), None),
                };
                let path = env.dir.join(&name);
                if name.ends_with(".csproj") {
                    proj.push_str(&format!("\n    <ProjectReference Include=\"{}\" />", path.display()));
                } else if name.ends_with(".dll") {
                    asm.push_str(&format!("\n    <Reference Include=\"{}\">\n      <HintPath>{}</HintPath>\n    </Reference>", path.file_stem().and_then(|s| s.to_str()).unwrap_or(""), path.display()));
                } else {
                    pkg.push_str(&format!("\n    <PackageReference Include={}{} />", prin1(&str(name.clone())), ver.map(|v| format!(" Version={}", prin1(&str(v)))).unwrap_or_default()));
                }
            }
            let grp = |s: String| if s.is_empty() { s } else { format!("<ItemGroup>{s}\n  </ItemGroup>") };
            format!("{}\n\n  {}\n\n  {}", grp(proj), grp(asm), grp(pkg))
        })
        .unwrap_or_default();
    let extra = opt("org-babel-csharp-additional-project-flags", "");
    let proj_file = base.join(format!("{project}.csproj"));
    write(
        &proj_file,
        &format!(
            "<Project Sdk=\"Microsoft.NET.Sdk\">\n\n  {refs}\n\n  <PropertyGroup>\n    <OutputType>Exe</OutputType>\n\n    <TargetFramework>{framework}</TargetFramework>\n    <ImplicitUsings>enable</ImplicitUsings>\n    <Nullable>enable</Nullable>{}\n  </PropertyGroup>\n</Project>",
            if extra.is_empty() || extra == "nil" {
                String::new()
            } else {
                format!("\n    {extra}")
            }
        ),
    )?;
    if let Some(n) = get_str(p, ":nugetconfig") {
        let src = env.dir.join(&n);
        if src.exists() {
            let _ = std::fs::copy(&src, base.join(src.file_name().unwrap_or_default()));
        }
    }
    env.eval(
        &format!(
            "{dotnet} restore {}",
            prin1(&str(proj_file.display().to_string()))
        ),
        "",
    )?;
    let compiled = env.eval(
        &format!(
            "{dotnet} build --output {} {}",
            prin1(&str(bin.display().to_string())),
            prin1(&str(proj_file.display().to_string()))
        ),
        "",
    )?;
    if compiled.contains(": error") {
        env.error_notify(Some(1), &compiled);
    }
    let out = env.eval(
        &format!(
            "{} {}",
            prin1(&str(bin.join(&project).display().to_string())),
            prin1(&str(get_str(p, ":cmdline").unwrap_or_default()))
        ),
        "",
    )?;
    let r = scalar_or_table(value::remove_indentation(&out), p, Sep::Auto)?;
    Ok(r.map(|r| reassemble(r, p)))
}

// ---- language metadata (org-src.el, ob-tangle.el) ----

/// org-babel-tangle-lang-exts.
pub fn tangle_ext(lang: &str) -> String {
    if let Some(v) = sexp::option("org-babel-tangle-lang-exts") {
        for e in items(&v).unwrap_or(&[]) {
            if let Some((k, x)) = value::car_cdr(e)
                && princ(&k) == lang
            {
                return items(&x)
                    .and_then(|i| i.first().map(princ))
                    .unwrap_or_else(|| princ(&x));
            }
        }
    }
    let exts = [
        ("emacs-lisp", "el"),
        ("elisp", "el"),
        ("bibtex", "bib"),
        ("awk", "awk"),
        ("C++", "cpp"),
        ("clojure", "clj"),
        ("clojurescript", "cljs"),
        ("csharp", "cs"),
        ("D", "d"),
        ("fortran", "F90"),
        ("groovy", "groovy"),
        ("haskell", "hs"),
        ("java", "java"),
        ("julia", "jl"),
        ("latex", "tex"),
        ("LilyPond", "ly"),
        ("lisp", "lisp"),
        ("lua", "lua"),
        ("maxima", "max"),
        ("ocaml", "ml"),
        ("perl", "pl"),
        ("processing", "pde"),
        ("python", "py"),
        ("ruby", "rb"),
        ("sed", "sed"),
    ];
    exts.iter()
        .find(|(l, _)| *l == lang)
        .map_or_else(|| lang.to_owned(), |(_, e)| (*e).to_owned())
}

/// org-src-get-lang-mode: the mode name for a language.
pub fn lang_mode(lang: &str) -> String {
    if let Some(v) = sexp::option("org-src-lang-modes") {
        for e in items(&v).unwrap_or(&[]) {
            if let Some((k, x)) = value::car_cdr(e)
                && princ(&k) == lang
            {
                return items(&x)
                    .and_then(|i| i.first().map(princ))
                    .unwrap_or_else(|| princ(&x));
            }
        }
    }
    match lang {
        "C" => "c",
        "C++" | "cpp" => "c++",
        "asymptote" => "asy",
        "beamer" => "latex",
        "calc" => "fundamental",
        "ditaa" => "artist",
        "desktop" => "conf-desktop",
        "dot" => "graphviz-dot",
        "elisp" => "emacs-lisp",
        "ocaml" => "tuareg",
        "screen" => "shell-script",
        "sqlite" => "sql",
        "toml" => "conf-toml",
        "shell" => "sh",
        l if shell_names().iter().any(|s| s == l) => "sh",
        l => l,
    }
    .to_owned()
}

/// comment-region in the language's mode then org-trim (noweb comments).
pub fn comment(lang: &str, text: &str) -> String {
    value::trim(&comment_region(lang, value::trim(text))).to_owned()
}

/// comment-region in the language's mode: non-blank lines commented.
pub fn comment_region(lang: &str, text: &str) -> String {
    let (start, end) = match lang_mode(lang).as_str() {
        "emacs-lisp" | "lisp" | "scheme" | "clojure" | "clojurescript" | "racket" => (";; ", ""),
        "c" | "css" => ("/* ", " */"),
        "c++" | "java" | "js" | "javascript" | "groovy" | "csharp" | "d" | "go" | "rust"
        | "scala" | "swift" | "kotlin" | "processing" | "typescript" => ("// ", ""),
        "lua" | "haskell" | "sql" | "ada" => ("-- ", ""),
        "latex" | "tex" | "matlab" | "octave" | "erlang" | "prolog" | "LilyPond" | "lilypond" => {
            ("% ", "")
        }
        "fortran" | "f90" => ("! ", ""),
        "ocaml" | "tuareg" | "fsharp" | "pascal" => ("(* ", " *)"),
        "html" | "xml" | "nxml" | "mhtml" | "markdown" => ("<!-- ", " -->"),
        "forth" => ("\\ ", ""),
        "maxima" => ("/* ", " */"),
        "fundamental" | "artist" | "plantuml" => ("", ""),
        "org" => ("# ", ""),
        _ => ("# ", ""),
    };
    if start.is_empty() {
        return text.to_owned();
    }
    text.split('\n')
        .map(|l| {
            if l.trim().is_empty() {
                l.to_owned()
            } else {
                format!("{start}{l}{end}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variable_assignments_like_upstream() {
        let t = sexp::read("((1 \"a\") (2 \"b\"))").unwrap();
        assert_eq!(python_var(&t), "[[1, \"a\"], [2, \"b\"]]");
        assert_eq!(python_var(&str("a\nb")), "\"\"\"a\nb\"\"\"");
        assert_eq!(ruby_var(&sexp::read("(1 2)").unwrap()), "[1, \n2]");
        assert_eq!(sh_var_to_sh(&t, None, None), "'1\ta\n2\tb'");
        assert_eq!(sh_var_to_sh(&str("it's"), None, None), "'it'\"'\"'s'");
        assert_eq!(octave_var(&t), "[[1,'a']; [2,'b']]");
        assert_eq!(
            elisp_expand(
                "x",
                &[(
                    ":var".into(),
                    value::cons(Sexp::Sym("x".into()), Sexp::Int(1))
                )]
            ),
            "(let ((x '1))\nx\n)"
        );
        assert_eq!(comment("emacs-lisp", "a\nb"), ";; a\n;; b");
        assert_eq!(
            python_shift_right("x = 1\ns = \"\"\"a\nb\"\"\"\ny", 4),
            "    x = 1\n    s = \"\"\"a\nb\"\"\"\n    y"
        );
        assert_eq!(result_cond(&["output".into()]), Some(true));
        assert_eq!(result_cond(&["replace".into()]), Some(false));
        assert_eq!(result_cond(&["discard".into()]), None);
    }
}
