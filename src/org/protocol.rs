//! org-protocol.el: `org-protocol://` URLs from a browser.
//!
//! Adaptation: Emacs receives these through emacsclient; Fred is started
//! with the URL as its file argument (`fred 'org-protocol://capture?...'`,
//! registered as the URL handler) or runs `:org-protocol URL`. Custom
//! sub-protocols in org-protocol-protocol-alist name a Fred Org command,
//! which gets the parameters as a `key=value&...` string.

use super::Prefix;
use super::sexp::Sexp;
use crate::editor::Editor;
use std::path::PathBuf;

/// Parameters as (key without the colon, value).
pub type Params = Vec<(String, String)>;

fn param<'a>(p: &'a Params, k: &str) -> Option<&'a str> {
    p.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.as_str())
}

/// org-link-decode: %XX escapes.
pub fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// org-protocol-sanitize-uri: `https:/x` back to `https://x`.
pub fn sanitize_uri(uri: &str) -> String {
    let re = crate::org_re!(r"^[a-z]+:/");
    if !re.is_match(uri) {
        return uri.to_owned();
    }
    let mut parts = crate::org_re!("/+").split(uri);
    let first = parts.next().unwrap_or_default();
    format!("{first}//{}", parts.collect::<Vec<_>>().join("/"))
}

/// org-protocol-split-data.
pub fn split_data(data: &str, unhexify: bool, sep: Option<&str>) -> Vec<String> {
    let sep = sep.unwrap_or(r"/+|\?");
    let re =
        super::re::compile(sep, false).unwrap_or_else(|_| regex::Regex::new(r"/+|\?").unwrap());
    re.split(data)
        .map(|p| if unhexify { decode(p) } else { p.to_owned() })
        .collect()
}

/// org-protocol-convert-query-to-plist (values decoded, as
/// org-protocol-parse-parameters does).
pub fn query_to_plist(q: &str) -> Params {
    q.replace('+', " ")
        .split('&')
        .map(|x| {
            let mut c = x.split('=');
            (
                c.next().unwrap_or_default().to_owned(),
                decode(c.next().unwrap_or_default()),
            )
        })
        .collect()
}

/// org-protocol-assign-parameters.
pub fn assign(data: Vec<String>, order: &[&str]) -> Params {
    let mut it = data.into_iter();
    let mut out: Params = order
        .iter()
        .map(|k| ((*k).to_owned(), it.next().unwrap_or_default()))
        .collect();
    while let Some(k) = it.next() {
        out.push((k, it.next().unwrap_or_default()));
    }
    out
}

/// org-protocol-parse-parameters.
pub fn parse(info: &str, new_style: bool, order: &[&str]) -> Params {
    if new_style {
        return query_to_plist(info);
    }
    let sep = super::options::string("org-protocol-data-separator", r"/+\|\?");
    let data = split_data(info, true, Some(&sep));
    assign(data, order)
}

/// org-protocol-store-link.
fn store_link(ed: &mut Editor, p: &Params) {
    let uri = sanitize_uri(param(p, "url").unwrap_or_default());
    let title = param(p, "title")
        .filter(|t| !t.is_empty())
        .map(str::to_owned);
    super::links::push_stored(uri.clone(), title);
    crate::vim::ops::set_reg(
        ed,
        crate::editor::Register {
            text: uri.clone(),
            linewise: false,
        },
    );
    ed.set_msg(format!(
        "`C-c C-l' to insert new Org link, `p' to insert \"{uri}\""
    ));
}

/// org-protocol-capture.
fn capture(ed: &mut Editor, info: &str, new_style: bool) -> Result<(), String> {
    let p = if new_style {
        parse(info, true, &[])
    } else {
        let data = parse(info, false, &[])
            .into_iter()
            .flat_map(|(k, v)| [k, v])
            .collect::<Vec<_>>();
        let data: Vec<String> = data.into_iter().filter(|s| !s.is_empty()).collect();
        let order: &[&str] = if data.first().is_some_and(|k| k.chars().count() == 1) {
            &["template", "url", "title", "body"]
        } else {
            &["url", "title", "body"]
        };
        assign(data, order)
    };
    let template = param(&p, "template")
        .map(str::to_owned)
        .or_else(|| {
            super::sexp::option("org-protocol-default-template-key")
                .and_then(|v| v.str().map(str::to_owned))
        })
        .filter(|t| t != "nil");
    let url = param(&p, "url").map(sanitize_uri);
    let kind = url
        .as_deref()
        .and_then(|u| crate::org_re!("^([a-z]+):").captures(u))
        .map(|c| c[1].to_owned());
    let title = param(&p, "title").unwrap_or_default().to_owned();
    let region = param(&p, "body").unwrap_or_default().to_owned();
    let annotation = match &url {
        None => title.clone(),
        Some(u) => {
            super::links::make_string(u, Some(if title.trim().is_empty() { u } else { &title }))?
        }
    };
    if let Some(u) = &url {
        super::links::push_stored(u.clone(), Some(title.clone()));
    }
    let mut props: Params = vec![
        ("type".into(), kind.unwrap_or_default()),
        ("link".into(), url.unwrap_or_default()),
        ("description".into(), title),
        ("annotation".into(), annotation),
        ("initial".into(), region),
    ];
    // :query parts are reachable as %:query too; keep the raw keys.
    props.extend(
        p.iter()
            .filter(|(k, _)| !props.iter().any(|(kk, _)| kk == k))
            .cloned()
            .collect::<Vec<_>>(),
    );
    super::capture::set_link_props(props);
    match template {
        Some(t) => super::call_with(ed, "org-capture", Prefix::None, t),
        None => super::call(ed, "org-capture", Prefix::None),
    }
}

/// org-protocol-open-source: the local file for a published URL.
fn open_source(p: &Params) -> Option<PathBuf> {
    let f = sanitize_uri(param(p, "url").unwrap_or_default());
    let alist = super::sexp::option("org-protocol-project-alist")?;
    for proj in alist.list().unwrap_or(&[]) {
        let plist = proj
            .list()
            .map(|l| l.get(1..).unwrap_or(&[]))
            .unwrap_or(&[]);
        let get = |k: &str| {
            plist
                .chunks(2)
                .find(|c| c[0].sym() == Some(k))
                .and_then(|c| c.get(1))
                .and_then(|v| v.str().map(str::to_owned))
        };
        let Some(base) = get(":base-url") else {
            continue;
        };
        let Some(at) = f.find(&base) else { continue };
        let wdir = get(":working-directory").unwrap_or_default();
        let f1 = &f[..f.find(['?', '#']).unwrap_or(f.len())];
        let start = at + base.len();
        let end = get(":online-suffix")
            .and_then(|s| f1.find(&s))
            .unwrap_or(f1.len());
        let f2 = format!("{wdir}{}", f1.get(start..end).unwrap_or_default());
        let file = super::options::expand(&match get(":working-suffix") {
            Some(s) => format!("{f2}{s}"),
            None => f2,
        });
        if file.exists() {
            return Some(file);
        }
        // :rewrites ((REGEXP . FILE)...): group 1 of REGEXP is dropped.
        let rewrites = plist
            .chunks(2)
            .find(|c| c[0].sym() == Some(":rewrites"))
            .and_then(|c| c.get(1).cloned());
        for rw in rewrites.as_ref().and_then(Sexp::list).unwrap_or(&[]) {
            let rep = rw.cdr();
            let (Some(re), Some(rep)) = (rw.car().and_then(Sexp::str), rep.str()) else {
                continue;
            };
            let Ok(re) = super::re::compile(re, false) else {
                continue;
            };
            if let Some(c) = re.captures(f1) {
                let g = c.get(1).or(c.get(0)).unwrap();
                let rest = format!("{}{}", &f1[..g.start()], &f1[g.end()..]);
                let dir = rest.trim_end_matches('/');
                return Some(super::options::expand(&format!("{wdir}{dir}{rep}")));
            }
        }
    }
    None
}

/// org-protocol-check-filename-for-protocol: run the handler for `fname`.
/// Ok(None) when it was handled; Ok(Some(file)) for a file to visit.
pub fn dispatch(ed: &mut Editor, fname: &str) -> Result<Option<PathBuf>, String> {
    let Some(m) = crate::org_re!("org-protocol:/+").find(fname) else {
        return Ok(Some(PathBuf::from(fname)));
    };
    let rest = &fname[m.end()..];
    let custom = super::sexp::option("org-protocol-protocol-alist");
    let mut subs: Vec<(String, String)> = custom
        .as_ref()
        .and_then(Sexp::list)
        .unwrap_or(&[])
        .iter()
        .filter_map(|e| {
            let pl = e.list()?.get(1..)?;
            let get = |k: &str| {
                pl.chunks(2)
                    .find(|c| c[0].sym() == Some(k))
                    .and_then(|c| c.get(1)?.str().map(str::to_owned))
            };
            Some((get(":protocol")?, get(":function")?))
        })
        .collect();
    for d in ["store-link", "capture", "open-source"] {
        subs.push((d.into(), format!("org-protocol-{d}")));
    }
    for (proto, func) in subs {
        let re = regex::Regex::new(&format!(r"^{}(:/+|/*\?)", regex::escape(&proto))).unwrap();
        let Some(c) = re.captures(rest) else { continue };
        let new_style = !c[1].starts_with(':');
        let info = &rest[c.get(0).unwrap().end()..];
        return match func.as_str() {
            "org-protocol-store-link" => {
                store_link(ed, &parse(info, new_style, &["url", "title"]));
                Ok(None)
            }
            "org-protocol-capture" => capture(ed, info, new_style).map(|()| None),
            "org-protocol-open-source" => match open_source(&parse(info, new_style, &["url"])) {
                Some(f) => Ok(Some(f)),
                None => Err(format!("No org-protocol-project-alist entry for {info}")),
            },
            other => super::call_with(ed, other, Prefix::None, info.to_owned()).map(|()| None),
        };
    }
    Ok(Some(PathBuf::from(fname)))
}

/// Startup with an org-protocol URL as the file argument.
pub fn is_protocol(arg: &str) -> bool {
    arg.contains("org-protocol:/")
}

pub fn command(ed: &mut Editor, name: &str, _arg: Prefix) -> Option<Result<(), String>> {
    Some(match name {
        "org-protocol" => {
            let Some(uri) = super::take_call_arg() else {
                super::read(ed, "org-protocol URL: ", "", |ed, u| {
                    if let Err(e) = super::call_with(ed, "org-protocol", Prefix::None, u) {
                        ed.set_err(e);
                    }
                });
                return Some(Ok(()));
            };
            match dispatch(ed, &uri) {
                Ok(Some(path)) => {
                    ed.pending_effect = Some(crate::ex::ExEffect::Open {
                        path,
                        line: 0,
                        col: 0,
                        pattern: None,
                    });
                    Ok(())
                }
                Ok(None) => Ok(()),
                Err(e) => Err(e),
            }
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing() {
        assert_eq!(
            sanitize_uri("https:/orgmode.org/worg"),
            "https://orgmode.org/worg"
        );
        assert_eq!(decode("a%20b%2Fc"), "a b/c");
        assert_eq!(
            query_to_plist("url=https%3A%2F%2Fx.org&title=A+B"),
            vec![
                ("url".into(), "https://x.org".into()),
                ("title".into(), "A B".into())
            ]
        );
        // Old style: org-protocol://store-link://URL/TITLE.
        let p = parse("https%3A%2F%2Fx.org%2Fa/Title", false, &["url", "title"]);
        assert_eq!(param(&p, "url"), Some("https://x.org/a"));
        assert_eq!(param(&p, "title"), Some("Title"));
    }

    #[test]
    fn store_link_url() {
        let mut e = super::super::tests::org("", "");
        assert_eq!(
            dispatch(
                &mut e,
                "org-protocol://store-link?url=https%3A%2F%2Fx.org&title=X"
            )
            .unwrap(),
            None
        );
        assert_eq!(
            super::super::links::stored_links()[0],
            ("https://x.org".into(), Some("X".into()))
        );
    }

    #[test]
    fn open_source_maps_url() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.org"), "").unwrap();
        let alist = format!(
            "((\"w\" :base-url \"https://w.org/\" :working-directory \"{}/\" :online-suffix \".html\" :working-suffix \".org\"))",
            dir.path().display()
        );
        super::super::options::put("org-protocol-project-alist", toml::Value::String(alist));
        let p = parse("url=https%3A%2F%2Fw.org%2Fa.html", true, &[]);
        assert_eq!(open_source(&p), Some(dir.path().join("a.org")));
        super::super::options::remove("org-protocol-project-alist");
    }
}
