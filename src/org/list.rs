//! Port of upstream Org: see docs/org-port-notes.md ("Module contracts").

use super::Prefix;
use crate::editor::Editor;

/// This module's interactive commands, by upstream name.
pub fn command(_ed: &mut Editor, _name: &str, _arg: Prefix) -> Option<Result<(), String>> {
    None
}

/// org-cycle on a list item (org-cycle-internal-local for items): None
/// when the cursor is not on an item's first line.
pub fn cycle_item(_ed: &mut Editor, _repeat: bool) -> Option<Result<(), String>> {
    None
}

/// org-cycle-item-indentation: TAB on an empty item cycles its level.
/// True when it acted.
pub fn cycle_item_indentation(_ed: &mut Editor) -> bool {
    false
}
