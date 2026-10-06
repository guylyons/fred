//! Golden reports from Emacs for every org-lint case of upstream's
//! test-org-lint.el (tools/org-lint-golden.el).

use super::*;

fn lint(text: &str, checkers: &[&'static str]) -> Vec<(usize, String)> {
    let st = super::super::settings_of(text, None);
    let t = element::parse_buffer(text, &st);
    let attach = |p: &str| PathBuf::from("/nonexistent-lint-dir/data").join(p);
    let none = |_: &str| false;
    let c = Ctx {
        text,
        t: &t,
        st: &st,
        dir: PathBuf::from("/nonexistent-lint-dir"),
        file: None,
        attach_expand: &attach,
        id_known: &none,
    };
    generate(&c, checkers)
        .into_iter()
        .map(|r| (r.line, r.message))
        .collect()
}

#[test]
fn golden_reports_match_emacs() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!("golden.json")).unwrap();
    let mut failures = vec![];
    for case in &cases {
        let text = case["text"].as_str().unwrap();
        let checkers: Vec<&'static str> = case["checkers"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| {
                CHECKERS
                    .iter()
                    .find(|x| x.0 == c.as_str().unwrap())
                    .map(|x| x.0)
            })
            .collect();
        let expected: Vec<(usize, String)> = case["expected"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r[0].as_u64().unwrap() as usize,
                    r[1].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        let got = lint(text, &checkers);
        if got != expected {
            failures.push(format!(
                "{checkers:?} on {text:?}\n  expected {expected:?}\n  got      {got:?}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases differ:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}
