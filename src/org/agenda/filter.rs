//! Agenda filters (org-agenda-filter-apply and its matchers): tag,
//! category, regexp, effort and top headline filters hide lines.

use super::{Item, View, opt_bool};

/// org-agenda-filter-make-matcher for the tag filter on `tags`.
fn tag_ok(filter: &[String], tags: &[String]) -> bool {
    filter.iter().all(|x| {
        let neg = x.starts_with('-');
        let tag = x.get(1..).unwrap_or("");
        let hit = if tag.is_empty() {
            !tags.is_empty()
        } else if tag.len() >= 2 && tag.starts_with('{') && tag.ends_with('}') {
            crate::org::re::compile(&tag[1..tag.len() - 1], false)
                .is_ok_and(|r| tags.iter().any(|t| r.is_match(t)))
        } else {
            tags.iter().any(|t| t == tag)
        };
        hit != neg
    })
}

fn category_ok(filter: &[String], cat: &str) -> bool {
    let multi_pos = filter.iter().filter(|x| x.starts_with('+')).count() > 1;
    let test = |x: &String| {
        let c = x.get(1..).unwrap_or("");
        if x.starts_with('-') {
            c != cat
        } else {
            c == cat
        }
    };
    if multi_pos {
        filter.iter().any(test)
    } else {
        filter.iter().all(test)
    }
}

fn regexp_ok(filter: &[String], txt: &str) -> bool {
    filter.iter().all(|x| {
        let neg = x.starts_with('-');
        let hit =
            crate::org::re::compile(x.get(1..).unwrap_or(""), true).is_ok_and(|r| r.is_match(txt));
        hit != neg
    })
}

/// org-agenda-filter-effort-form / org-agenda-compare-effort.
fn effort_ok(filter: &[String], it: &Item) -> bool {
    let high = opt_bool("org-agenda-sort-noeffort-is-high", true);
    let e = it
        .effort_minutes
        .unwrap_or(if high { f64::MAX } else { -1.0 });
    filter.iter().all(|x| {
        let rest = x.get(1..).unwrap_or("");
        let op = rest.chars().next().unwrap_or('=');
        let v = crate::org::time::duration::to_minutes(rest.get(1..).unwrap_or(""), false)
            .unwrap_or(0.0);
        match op {
            '<' => e <= v,
            '>' => e >= v,
            _ => (e - v).abs() < f64::EPSILON,
        }
    })
}

fn merged(preset: &[String], f: &[String]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for x in preset.iter().chain(f) {
        if !out.contains(x) {
            out.push(x.clone());
        }
    }
    out
}

/// Hide the lines the current filters reject (all filter types).
pub fn apply(v: &mut View) {
    let tag = merged(&v.presets.tag, &v.filters.tag);
    let cat = merged(&v.presets.category, &v.filters.category);
    let re = merged(&v.presets.regexp, &v.filters.regexp);
    let eff = merged(&v.presets.effort, &v.filters.effort);
    let top = v.filters.top.clone();
    for l in &mut v.lines {
        l.hidden.retain(|t| {
            !matches!(
                *t,
                "tag" | "category" | "regexp" | "effort" | "top-headline"
            )
        });
        let Some(it) = &l.item else { continue };
        if it.marker.is_none() && it.hd.is_none() {
            continue;
        }
        let tags = it.tag_names();
        if !tag.is_empty() && !tag_ok(&tag, &tags) {
            l.hidden.push("tag");
        }
        if !cat.is_empty() && !category_ok(&cat, &it.category) {
            l.hidden.push("category");
        }
        if !re.is_empty() && !regexp_ok(&re, &it.txt) {
            l.hidden.push("regexp");
        }
        if !eff.is_empty() && !effort_ok(&eff, it) {
            l.hidden.push("effort");
        }
        if let Some((hl, neg)) = &top
            && !it.top.is_empty()
            && (it.top == *hl) == *neg
        {
            l.hidden.push("top-headline");
        }
    }
}

/// The categories shown in the view (org-agenda-get-represented-categories).
pub fn represented_categories(v: &View) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for l in &v.lines {
        if let Some(it) = &l.item
            && !it.category.is_empty()
        {
            let c = if it.category.contains('-') {
                format!("\"{}\"", it.category)
            } else {
                it.category.clone()
            };
            if !out.contains(&c) {
                out.push(c);
            }
        }
    }
    out
}

/// The tags shown in the view (org-agenda-get-represented-tags).
pub fn represented_tags(v: &View) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for l in &v.lines {
        if let Some(it) = &l.item {
            for t in it.tag_names() {
                if !out.contains(&t) {
                    out.push(t);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn matchers() {
        assert!(tag_ok(&s(&["+work"]), &s(&["work", "x"])));
        assert!(!tag_ok(&s(&["+work"]), &s(&["home"])));
        assert!(tag_ok(&s(&["-work"]), &s(&["home"])));
        assert!(tag_ok(&s(&["+{^wo}"]), &s(&["work"])));
        assert!(category_ok(&s(&["+a", "+b"]), "b"));
        assert!(!category_ok(&s(&["+a"]), "b"));
        assert!(category_ok(&s(&["-a"]), "b"));
        assert!(regexp_ok(&s(&["+REP"]), "Write report"));
        assert!(!regexp_ok(&s(&["-rep"]), "Write report"));
        let it = Item {
            effort_minutes: Some(30.0),
            ..Item::default()
        };
        assert!(effort_ok(&s(&["+<1:00"]), &it));
        assert!(!effort_ok(&s(&["+>1:00"]), &it));
    }
}
