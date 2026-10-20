//! Port of upstream Org: see docs/org-port-notes.md ("Module contracts").

use super::Prefix;
use crate::editor::Editor;

/// This module's interactive commands, by upstream name.
pub fn command(_ed: &mut Editor, _name: &str, _arg: Prefix) -> Option<Result<(), String>> {
    None
}
