//! The `:` command line: ed addresses and commands.

pub mod addr;
pub mod cmd;

pub use cmd::{BufCmd, ExEffect, ExState, run, unfence};
