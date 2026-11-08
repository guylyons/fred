//! Semantic decoration hints for the native GUI. Never changes buffer text.
use super::RowAction;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Role {
    Heading { open: bool },
    File,
    HunkHeader,
    Added,
    Removed,
    Context,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    Count,
    RefLocal,
    RefRemote,
    Status,
    Key,
    Group,
    OptOn,
    OptOff,
}
#[derive(Default, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Paint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<(u16, u16, Mark)>,
}

pub fn paint_rows(
    action_at: impl Fn(usize) -> Option<RowAction>,
    line: impl Fn(usize) -> String,
    from: usize,
    to: usize,
) -> Vec<Paint> {
    fn body(s: &str) -> bool {
        !s.starts_with("+++") && !s.starts_with("---") && s.starts_with(['+', '-', ' ', '\\'])
    }
    let mut in_hunk = false;
    for i in (from.saturating_sub(5000)..from).rev() {
        let s = line(i);
        if s.starts_with("@@") {
            in_hunk = true;
            break;
        }
        if action_at(i).is_some_and(|a| matches!(a, RowAction::Section(_) | RowAction::File(..)))
            || !body(&s)
        {
            break;
        }
    }
    (from..to)
        .map(|i| {
            let text = line(i);
            let mut p = Paint::default();
            match action_at(i) {
                Some(RowAction::Section(_)) => {
                    in_hunk = false;
                    p.role = Some(Role::Heading {
                        open: !text.ends_with('…'),
                    });
                    let s = text.trim_end_matches('…');
                    if let Some(at) = s.rfind('(') {
                        let digits = s[at..].strip_prefix('(').and_then(|s| s.strip_suffix(')'));
                        if digits
                            .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                        {
                            p.marks.push((
                                s[..at].chars().count() as u16,
                                s.chars().count() as u16,
                                Mark::Count,
                            ));
                        }
                    }
                }
                Some(RowAction::File(..)) => {
                    in_hunk = false;
                    p.role = Some(Role::File);
                    for status in [
                        "modified",
                        "new file",
                        "deleted",
                        "renamed",
                        "untracked",
                        "unmerged",
                        "copied",
                        "typechange",
                    ] {
                        if text
                            .strip_prefix(status)
                            .is_some_and(|s| s.starts_with(' '))
                        {
                            p.marks.push((0, status.len() as u16, Mark::Status));
                            break;
                        }
                    }
                }
                _ if text.starts_with("@@") => {
                    in_hunk = true;
                    p.role = Some(Role::HunkHeader);
                }
                _ if in_hunk && body(&text) => {
                    p.role = match text.chars().next() {
                        Some('+') => Some(Role::Added),
                        Some('-') => Some(Role::Removed),
                        _ => Some(Role::Context),
                    };
                }
                _ => in_hunk = false,
            }
            let kind = if ["Head:", "Merge:", "Rebase:"]
                .iter()
                .any(|s| text.starts_with(s))
            {
                Some(Mark::RefLocal)
            } else if ["Push:", "Upstream:"].iter().any(|s| text.starts_with(s)) {
                Some(Mark::RefRemote)
            } else {
                None
            };
            if let Some(kind) = kind {
                let n = text
                    .chars()
                    .skip(10)
                    .take_while(|c| !c.is_whitespace())
                    .count();
                if n > 0 {
                    p.marks.push((10, (10 + n) as u16, kind));
                }
            }
            p
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::magit::Section;
    fn paint(lines: &[&str], actions: &[(usize, RowAction)], from: usize, to: usize) -> Vec<Paint> {
        paint_rows(
            |i| {
                actions
                    .iter()
                    .find(|(n, _)| *n == i)
                    .map(|(_, a)| a.clone())
            },
            |i| lines[i].into(),
            from,
            to,
        )
    }
    #[test]
    fn headings() {
        let p = paint(
            &[
                "Unstaged changes (2)",
                "Untracked files (1)…",
                "Recent commits",
            ],
            &(0..3)
                .map(|i| (i, RowAction::Section(Section::Unstaged)))
                .collect::<Vec<_>>(),
            0,
            3,
        );
        assert_eq!(p[0].role, Some(Role::Heading { open: true }));
        assert_eq!(p[0].marks, [(17, 20, Mark::Count)]);
        assert_eq!(p[1].role, Some(Role::Heading { open: false }));
        assert_eq!(p[1].marks, [(16, 19, Mark::Count)]);
        assert!(p[2].marks.is_empty());
    }
    #[test]
    fn hunk_rows_and_mid_hunk() {
        let lines = [
            "modified   src/a.rs",
            "@@ -1,2 +1,2 @@",
            " ctx",
            "-old",
            "+new",
            "Recent commits",
        ];
        let actions = [
            (0, RowAction::File("src/a.rs".into(), Section::Unstaged)),
            (5, RowAction::Section(Section::Unstaged)),
        ];
        let p = paint(&lines, &actions, 0, 6);
        assert_eq!(
            p.iter().map(|p| p.role.unwrap()).collect::<Vec<_>>(),
            [
                Role::File,
                Role::HunkHeader,
                Role::Context,
                Role::Removed,
                Role::Added,
                Role::Heading { open: true }
            ]
        );
        assert_eq!(p[0].marks, [(0, 8, Mark::Status)]);
        assert_eq!(paint(&lines, &actions, 3, 5), p[3..5]);
    }
    #[test]
    fn headers_and_log_body_are_not_hunks() {
        for lines in [["+++ b/x", "--- a/x"], ["abc1234 subject", "    body line"]] {
            assert!(paint(&lines, &[], 0, 2).iter().all(|p| p.role.is_none()));
        }
        let p = paint(
            &["@@ -1 +1 @@", "+new", "--- a/x", "+++ b/x", " body"],
            &[],
            0,
            5,
        );
        assert!(p[2..].iter().all(|p| p.role.is_none()));
    }
    #[test]
    fn header_refs() {
        let p = paint(
            &["Head:     main subject", "Push:     origin/main subject"],
            &[],
            0,
            2,
        );
        assert_eq!(p[0].marks, [(10, 14, Mark::RefLocal)]);
        assert_eq!(p[1].marks, [(10, 21, Mark::RefRemote)]);
    }
}
