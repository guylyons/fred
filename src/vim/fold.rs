//! Vim's fold commands (`zo`, `zc`, `za`, `zR`, `zM`, …) for code buffers.

use super::motion::Motion;
use crate::editor::Editor;
use crate::options::{Level, Opt, Value};

/// 'foldopen' at Neovim's default (block, hor, mark, percent, search): a
/// motion that opens the folds it lands in. `same_line`: the cursor stayed
/// on its line.
pub fn opens(m: Motion, same_line: bool) -> bool {
    match m {
        Motion::Up | Motion::Down => false,
        Motion::Percent
        | Motion::SearchNext { .. }
        | Motion::Star { .. }
        | Motion::Mark(_)
        | Motion::MarkExact(_)
        | Motion::Unmatched(_)
        | Motion::Sentence(_)
        | Motion::ParaFwd
        | Motion::ParaBack
        | Motion::Section { .. } => true,
        _ => same_line,
    }
}

/// The `z` commands that are fold commands.
pub const KEYS: &str = "oOcCaAvxXmMrRnNijkfFdDE";

pub fn key(ed: &mut Editor, c: char, count: Option<usize>) {
    // Org folds by its own rules (TAB); special buffers have no folds.
    if !ed.has_code_folds() {
        return;
    }
    let (o, l, n) = (ed.fold_opts(), ed.cur.line, count.unwrap_or(1).max(1));
    let level = |ed: &mut Editor, v: usize| {
        let _ = ed.opts.put(Opt::Foldlevel, Value::Num(v as i64), Level::Local);
        let _ = ed.opts.put(Opt::Foldenable, Value::Bool(true), Level::Local);
        ed.code_folds.reset();
    };
    let enable = |ed: &mut Editor, v: bool| {
        let _ = ed.opts.put(Opt::Foldenable, Value::Bool(v), Level::Local);
    };
    let found = match c {
        'o' => ed.code_folds.open(l, n, false, o),
        'O' => ed.code_folds.open(l, 1, true, o),
        'c' => ed.code_folds.close(l, n, false, o),
        'C' => ed.code_folds.close(l, 1, true, o),
        'a' | 'A' if ed.code_folds.closed_at(l, o).is_some() => {
            ed.code_folds.open(l, n, c == 'A', o)
        }
        'a' | 'A' => ed.code_folds.close(l, n, c == 'A', o),
        'v' => {
            ed.code_folds.view(l, o);
            true
        }
        'x' | 'X' => {
            ed.code_folds.reset();
            // Compute the folds again too.
            ed.code_folds.synced = None;
            if c == 'x' {
                ed.code_folds.view(l, o);
            }
            true
        }
        'm' => {
            level(ed, o.level.saturating_sub(n));
            true
        }
        'M' => {
            level(ed, 0);
            true
        }
        'r' => {
            let deepest = ed.code_folds.deepest();
            level(ed, (o.level + n).min(deepest.max(o.level)));
            true
        }
        'R' => {
            let deepest = ed.code_folds.deepest();
            level(ed, deepest);
            true
        }
        'n' | 'N' => {
            enable(ed, c == 'N');
            true
        }
        'i' => {
            enable(ed, !o.enable);
            true
        }
        'j' | 'k' => {
            for _ in 0..n {
                let l = ed.cur.line;
                let to = if c == 'j' {
                    ed.code_folds.next_start(l)
                } else {
                    ed.code_folds.prev_end(l)
                };
                if let Some(to) = to {
                    ed.set_line_keep_col(to);
                }
            }
            true
        }
        'f' | 'F' => return ed.set_err("E350: Cannot create fold with current 'foldmethod'"),
        'd' | 'D' => return ed.set_err("E351: Cannot delete fold with current 'foldmethod'"),
        'E' => return ed.set_err("E352: Cannot erase folds with current 'foldmethod'"),
        _ => return,
    };
    if !found {
        return ed.set_err("E490: No fold found");
    }
    ed.apply_code_folds();
}
