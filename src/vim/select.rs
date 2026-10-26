//! Neovim's Select mode (`:h Select-mode`) and 'keymodel': a Visual
//! selection (`ed.vim.select`) that typing replaces, as in other editors.

use super::{insert, ops};
use crate::editor::{Editor, Mode};
use crate::key::{Key, KeyCode};
use crate::options::Opt;

/// The keys 'keymodel' starts (shifted) or stops (unshifted) a selection with.
fn special(k: Key) -> bool {
    !k.ctrl
        && !k.alt
        && matches!(
            k.code,
            KeyCode::Left
                | KeyCode::Right
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::PageUp
                | KeyCode::PageDown
        )
}

/// A Shift key that 'keymodel' startsel makes select (in Normal, Insert,
/// Visual and Select mode).
pub(crate) fn shift_selects(ed: &Editor, k: Key) -> bool {
    special(k)
        && k.shift
        && ed.opts.has(Opt::Keymodel, "startsel")
        && matches!(
            ed.mode,
            Mode::Normal | Mode::Insert | Mode::Visual { .. } | Mode::VisualLine { .. }
        )
}

/// Start a selection from `anchor` to the cursor, in Select mode when
/// `select`. From Insert mode (like Insert CTRL-O) the insert closes and
/// the selection goes back to Insert when it ends without a change.
pub(crate) fn start(ed: &mut Editor, anchor: (usize, usize), linewise: bool, select: bool) {
    ed.vim.restart_insert = ed.mode == Mode::Insert;
    if ed.mode == Mode::Insert {
        // Up and Down keep the column Insert left the cursor in.
        let (l, b) = ed.cur.pos();
        ed.set_cursor(l, b);
        ed.vim.after_j = false;
        ed.popup = None;
        ed.undo.end(ed.cur.pos());
        if let Some(r) = ed.vim.recording.take() {
            ed.vim.finish_recording(r);
        }
        ed.vim.insert_left_eol =
            (ed.cur.byte >= ed.buf.line_len(ed.cur.line)).then_some(ed.cur.line);
    }
    ed.mode = if linewise {
        Mode::VisualLine { anchor: anchor.0 }
    } else {
        Mode::Visual { anchor }
    };
    ed.vim.select = select;
}

/// End the selection: back to Insert if it began there, else Normal.
pub(crate) fn end(ed: &mut Editor) {
    ed.mode = Mode::Normal;
    ed.vim.select = false;
    if std::mem::take(&mut ed.vim.restart_insert) {
        ed.mode = Mode::Insert;
        // Insert was left at the end of this line: on its last character,
        // go back after it (Neovim's `ins_at_eol`).
        let (l, b) = ed.cur.pos();
        let len = ed.buf.line_len(l);
        if ed.vim.insert_left_eol.take() == Some(l)
            && b < len
            && crate::text::next_grapheme(&ed.buf.line(l), b) == len
        {
            ed.cur.byte = len;
        }
        ed.undo.begin(ed.cur.pos());
    }
}

/// Select-mode and 'keymodel' keys in Normal, Visual and Select mode, with
/// nothing pending. Returns the key to go on with (startsel unshifts it),
/// or None when it was handled here.
pub(crate) fn key(ed: &mut Editor, k: Key) -> Option<Key> {
    let visual = ed.visual_lines().is_some();
    if shift_selects(ed, k) {
        if !visual {
            let select = ed.opts.has(Opt::Selectmode, "key");
            start(ed, ed.cur.pos(), false, select);
        }
        return Some(Key { shift: false, ..k });
    }
    if !visual {
        return Some(k);
    }
    if special(k) && !k.shift && ed.opts.has(Opt::Keymodel, "stopsel") {
        end(ed);
        if ed.mode == Mode::Insert {
            insert::insert_key(ed, k);
            return None;
        }
        return Some(k);
    }
    if k.is(KeyCode::Esc) || k == Key::ctrl('c') {
        end(ed);
        return None;
    }
    if k == Key::ctrl('g') {
        ed.vim.select = !ed.vim.select;
        return None;
    }
    if !ed.vim.select {
        return Some(k);
    }
    match k.code {
        KeyCode::Backspace | KeyCode::Delete if !k.ctrl && !k.alt => delete(ed),
        KeyCode::Char('h') if k.ctrl => delete(ed),
        KeyCode::Char(_) | KeyCode::Enter if !k.ctrl && !k.alt => {
            if change(ed, false) {
                insert::insert_key(ed, k);
            }
        }
        _ => return Some(k),
    }
    None
}

/// Shift+arrow in Insert mode with 'keymodel' startsel: select from the
/// cursor (and come back to Insert afterwards). True when handled.
pub(crate) fn insert_key(ed: &mut Editor, k: Key) -> bool {
    if !shift_selects(ed, k) {
        return false;
    }
    let select = ed.opts.has(Opt::Selectmode, "key");
    start(ed, ed.cur.pos(), false, select);
    // Still shifted, so stopsel leaves it alone.
    super::normal_key(ed, k);
    true
}

/// The selection as a span, leaving the selection. A linewise selection is
/// changed like a charwise one (`change`), as in Neovim.
fn take(ed: &mut Editor, change: bool) -> Option<ops::Span> {
    let span = super::selection(ed, &ed.mode)?;
    ed.mode = Mode::Normal;
    ed.vim.select = false;
    if ed.generated() {
        ed.vim.restart_insert = false;
        ed.set_err("generated Git buffer is read-only");
        return None;
    }
    Some(match span {
        ops::Span::Lines(lo, hi) if change => {
            let to = if hi + 1 < ed.line_count() {
                (hi + 1, 0)
            } else {
                (hi, ed.buf.line_len(hi))
            };
            ops::Span::Chars((lo, 0), to)
        }
        s => s,
    })
}

/// `op` on `span` into the unnamed register only: Fred's deliberate
/// difference from Neovim, whose 'clipboard' would sync it (see
/// docs/nvim-parity.csv, `gh`).
fn apply_keeping_clipboard(ed: &mut Editor, op: char, span: ops::Span) {
    ed.vim.keep_clipboard = true;
    ops::apply_op(ed, op, span);
    ed.vim.keep_clipboard = false;
}

fn start_of(span: &ops::Span) -> (usize, usize) {
    match *span {
        ops::Span::Chars(from, _) => from,
        ops::Span::Lines(lo, _) => (lo, 0),
    }
}

/// Select `<BS>`/`<Del>`: delete the selection. Insert mode again if the
/// selection began there.
fn delete(ed: &mut Editor) {
    let Some(span) = take(ed, false) else { return };
    ed.undo.begin(start_of(&span));
    apply_keeping_clipboard(ed, 'd', span);
    ed.undo.end(ed.cur.pos());
    end(ed);
}

/// Typing in Select mode: change the selection and stay in Insert (the
/// undo group stays open until Insert ends); `black_hole` leaves the
/// registers alone (a paste). False if nothing changed.
pub(crate) fn change(ed: &mut Editor, black_hole: bool) -> bool {
    let Some(span) = take(ed, true) else {
        return false;
    };
    ed.vim.restart_insert = false;
    let from = start_of(&span);
    ed.undo.begin(from);
    // Undo comes back to the selection's start, as Vim's.
    ed.set_cursor(from.0, from.1);
    if black_hole {
        ops::delete_black_hole(ed, span);
        ed.mode = Mode::Insert;
        ed.set_cursor(from.0, from.1);
    } else {
        apply_keeping_clipboard(ed, 'c', span);
    }
    true
}
