//! Neovim's options (`:h options`): one table, as `src/nvim/options.lua`
//! upstream, and the values set from it.
//!
//! A global option has one value shared by every buffer. A buffer or window
//! option also has a local value per `Editor` (which is both a buffer and a
//! window until windows split off): `:set` changes both, `:setlocal` the
//! local one, `:setglobal` the global one that new buffers start from.

mod set;

pub use set::{Level, set};

use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Global,
    Buffer,
    Window,
}

/// A default as written in the table.
#[derive(Clone, Copy, Debug)]
pub enum DefaultValue {
    Bool(bool),
    Num(i64),
    Str(&'static str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Bool(bool),
    Num(i64),
    Str(String),
}

impl From<DefaultValue> for Value {
    fn from(d: DefaultValue) -> Value {
        match d {
            DefaultValue::Bool(b) => Value::Bool(b),
            DefaultValue::Num(n) => Value::Num(n),
            DefaultValue::Str(s) => Value::Str(s.into()),
        }
    }
}

pub struct Def {
    pub name: &'static str,
    pub abbrev: &'static str,
    pub scope: Scope,
    pub default: DefaultValue,
    /// A comma-separated list: `+=` and `-=` add and remove items.
    pub list: bool,
    /// Rejects a value the option can't take.
    check: fn(&Value) -> Result<(), String>,
}

/// A table entry: `opt("tabstop", "ts", Buffer, Num(8)).check(positive)`.
const fn opt(name: &'static str, abbrev: &'static str, scope: Scope, default: DefaultValue) -> Def {
    Def {
        name,
        abbrev,
        scope,
        default,
        list: false,
        check: |_| Ok(()),
    }
}

impl Def {
    const fn list(self) -> Def {
        Def { list: true, ..self }
    }

    const fn check(self, check: fn(&Value) -> Result<(), String>) -> Def {
        Def { check, ..self }
    }
}

macro_rules! options {
    ($($(#[$doc:meta])* $id:ident => $def:expr,)*) => {
        /// An option, by the name Fred's code reads it with.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Opt {
            $($(#[$doc])* $id,)*
        }

        /// Every option Fred implements, in `Opt` order.
        pub const OPTIONS: &[Def] = &[$($def,)*];

        impl Opt {
            const ALL: &[Opt] = &[$(Opt::$id,)*];
        }
    };
}

use DefaultValue::{Bool, Num, Str};
use Scope::{Buffer, Global, Window};

// Where Fred's default differs from Neovim's, Neovim's is noted.
options! {
    /// Complete words while typing (Neovim: off).
    Autocomplete => opt("autocomplete", "ac", Global, Bool(true)),
    /// Yank to and put from the system clipboard. Fred's config turns it
    /// on (`clipboard = true`).
    Clipboard => opt("clipboard", "cb", Global, Str(""))
        .list()
        .check(|v| items(v, &["unnamed", "unnamedplus"])),
    /// Highlight the cursor's line.
    Cursorline => opt("cursorline", "cul", Window, Bool(false)),
    /// Line numbers (Neovim: off).
    Number => opt("number", "nu", Window, Bool(true)),
    /// Line numbers relative to the cursor's.
    Relativenumber => opt("relativenumber", "rnu", Window, Bool(false)),
    /// Columns a tab takes.
    Tabstop => opt("tabstop", "ts", Buffer, Num(8)).check(positive),
    /// Wrap long lines (Neovim: on).
    Wrap => opt("wrap", "", Window, Bool(false)),
}

fn positive(v: &Value) -> Result<(), String> {
    match v {
        Value::Num(n) if *n > 0 => Ok(()),
        _ => Err("E487: Argument must be positive".into()),
    }
}

/// Every item of a list value is one of `allowed`.
fn items(v: &Value, allowed: &[&str]) -> Result<(), String> {
    let Value::Str(s) = v else { return Ok(()) };
    match s.split(',').find(|i| !i.is_empty() && !allowed.contains(i)) {
        Some(bad) => Err(format!("E474: Invalid argument: {bad}")),
        None => Ok(()),
    }
}

impl Opt {
    pub fn def(self) -> &'static Def {
        &OPTIONS[self as usize]
    }

    /// The option `name` names or abbreviates.
    pub fn find(name: &str) -> Option<Opt> {
        let i = OPTIONS
            .iter()
            .position(|d| d.name == name || (!d.abbrev.is_empty() && d.abbrev == name))?;
        Some(Opt::ALL[i])
    }
}

/// Option values: global ones shared by every buffer, plus this buffer's and
/// window's own.
#[derive(Debug)]
pub struct Options {
    global: Rc<RefCell<Vec<Value>>>,
    /// Some for buffer and window options.
    local: Vec<Option<Value>>,
}

impl Default for Options {
    fn default() -> Options {
        let global: Vec<Value> = OPTIONS.iter().map(|d| d.default.into()).collect();
        let local = OPTIONS
            .iter()
            .zip(&global)
            .map(|(d, v)| (d.scope != Scope::Global).then(|| v.clone()))
            .collect();
        Options {
            global: Rc::new(RefCell::new(global)),
            local,
        }
    }
}

impl Options {
    /// Another buffer shown in this window: it shares the global values,
    /// starts its buffer options from them and keeps this window's options.
    pub fn for_new_buffer(&self) -> Options {
        let global = self.global.borrow();
        let local = OPTIONS
            .iter()
            .enumerate()
            .map(|(i, d)| match d.scope {
                Scope::Global => None,
                Scope::Buffer => Some(global[i].clone()),
                Scope::Window => self.local[i].clone(),
            })
            .collect();
        Options {
            global: self.global.clone(),
            local,
        }
    }

    /// The value in effect here.
    pub fn get(&self, o: Opt) -> Value {
        self.get_at(o, Level::Both)
    }

    /// The global value (`Level::Global`), or the one in effect here.
    pub fn get_at(&self, o: Opt, level: Level) -> Value {
        let i = o as usize;
        match (&self.local[i], level) {
            (Some(v), Level::Both | Level::Local) => v.clone(),
            _ => self.global.borrow()[i].clone(),
        }
    }

    pub fn bool(&self, o: Opt) -> bool {
        matches!(self.get(o), Value::Bool(true))
    }

    pub fn num(&self, o: Opt) -> i64 {
        match self.get(o) {
            Value::Num(n) => n,
            _ => 0,
        }
    }

    pub fn str(&self, o: Opt) -> String {
        match self.get(o) {
            Value::Str(s) => s,
            _ => String::new(),
        }
    }

    /// Set `o` at `level`; a global option has only its global value.
    pub fn put(&mut self, o: Opt, v: Value, level: Level) -> Result<(), String> {
        (o.def().check)(&v)?;
        let i = o as usize;
        let global = o.def().scope == Scope::Global;
        if global || level != Level::Local {
            self.global.borrow_mut()[i] = v.clone();
        }
        if !global && level != Level::Global {
            self.local[i] = Some(v);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
