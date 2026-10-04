//! Emacs Lisp data: option values, header arguments, `#+BIND`. A reader
//! for the printed representation, and the mapping from TOML (arrays are
//! lists, strings are strings or symbols, booleans are t/nil).

use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Sexp {
    Nil,
    T,
    Int(i64),
    Float(f64),
    Str(String),
    Sym(String),
    List(Vec<Sexp>),
    /// A dotted pair's last cdr: `(a b . c)` is `Dotted([a, b], c)`.
    Dotted(Vec<Sexp>, Box<Sexp>),
    Vector(Vec<Sexp>),
}

impl Sexp {
    pub fn is_nil(&self) -> bool {
        matches!(self, Sexp::Nil) || matches!(self, Sexp::List(v) if v.is_empty())
    }

    /// A list's elements (nil is empty; an atom is not a list).
    pub fn list(&self) -> Option<&[Sexp]> {
        match self {
            Sexp::List(v) | Sexp::Vector(v) => Some(v),
            Sexp::Nil => Some(&[]),
            _ => None,
        }
    }

    /// A string or a symbol's name.
    pub fn str(&self) -> Option<&str> {
        match self {
            Sexp::Str(s) | Sexp::Sym(s) => Some(s),
            _ => None,
        }
    }

    pub fn sym(&self) -> Option<&str> {
        match self {
            Sexp::Sym(s) => Some(s),
            Sexp::T => Some("t"),
            Sexp::Nil => Some("nil"),
            _ => None,
        }
    }

    pub fn int(&self) -> Option<i64> {
        match self {
            Sexp::Int(i) => Some(*i),
            Sexp::Float(f) => Some(*f as i64),
            _ => None,
        }
    }

    /// `(car . cdr)` of a cons: a list's head and rest, or a dotted pair.
    pub fn car(&self) -> Option<&Sexp> {
        match self {
            Sexp::List(v) => v.first(),
            Sexp::Dotted(v, _) => v.first(),
            _ => None,
        }
    }

    /// The cdr of an alist entry `(key . value)`: the value for a dotted
    /// pair, else the rest as a list.
    pub fn cdr(&self) -> Sexp {
        match self {
            Sexp::Dotted(v, tail) if v.len() == 1 => (**tail).clone(),
            Sexp::Dotted(v, tail) => Sexp::Dotted(v[1..].to_vec(), tail.clone()),
            Sexp::List(v) if v.len() > 1 => Sexp::List(v[1..].to_vec()),
            _ => Sexp::Nil,
        }
    }

    /// `(plist-get LIST :key)`.
    pub fn plist_get(&self, key: &str) -> Option<&Sexp> {
        let v = self.list()?;
        v.iter()
            .position(|s| s.sym() == Some(key))
            .and_then(|i| v.get(i + 1))
    }

    pub fn truthy(&self) -> bool {
        !self.is_nil()
    }

    pub fn from_toml(v: &toml::Value) -> Sexp {
        match v {
            toml::Value::Boolean(true) => Sexp::T,
            toml::Value::Boolean(false) => Sexp::Nil,
            toml::Value::Integer(i) => Sexp::Int(*i),
            toml::Value::Float(f) => Sexp::Float(*f),
            toml::Value::String(s) => {
                // A Lisp form written as a string: '(...) or (...).
                if let Some(rest) = s.strip_prefix('\'') {
                    return read(rest).unwrap_or_else(|_| Sexp::Str(s.clone()));
                }
                if s.starts_with('(') {
                    return read(s).unwrap_or_else(|_| Sexp::Str(s.clone()));
                }
                Sexp::Str(s.clone())
            }
            toml::Value::Array(a) => {
                // ["a", "b", ".", "c"] is the dotted list (a b . c).
                let items: Vec<Sexp> = a.iter().map(Sexp::from_toml).collect();
                let n = items.len();
                if n >= 3 && items[n - 2] == Sexp::Str(".".into()) {
                    let mut items = items;
                    let tail = items.pop().unwrap();
                    items.pop();
                    return Sexp::Dotted(items, Box::new(tail));
                }
                Sexp::List(items)
            }
            toml::Value::Table(t) => Sexp::List(
                t.iter()
                    .map(|(k, v)| Sexp::Dotted(vec![Sexp::Str(k.clone())], Box::new(Sexp::from_toml(v))))
                    .collect(),
            ),
            toml::Value::Datetime(d) => Sexp::Str(d.to_string()),
        }
    }
}

impl fmt::Display for Sexp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Sexp::Nil => write!(f, "nil"),
            Sexp::T => write!(f, "t"),
            Sexp::Int(i) => write!(f, "{i}"),
            Sexp::Float(x) => write!(f, "{x}"),
            Sexp::Str(s) => {
                write!(f, "\"")?;
                for c in s.chars() {
                    match c {
                        '"' => write!(f, "\\\"")?,
                        '\\' => write!(f, "\\\\")?,
                        c => write!(f, "{c}")?,
                    }
                }
                write!(f, "\"")
            }
            Sexp::Sym(s) => write!(f, "{s}"),
            Sexp::List(v) => {
                write!(f, "(")?;
                for (i, x) in v.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{x}")?;
                }
                write!(f, ")")
            }
            Sexp::Dotted(v, t) => {
                write!(f, "(")?;
                for x in v {
                    write!(f, "{x} ")?;
                }
                write!(f, ". {t})")
            }
            Sexp::Vector(v) => {
                write!(f, "[")?;
                for (i, x) in v.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{x}")?;
                }
                write!(f, "]")
            }
        }
    }
}

/// Read one form from `s`.
pub fn read(s: &str) -> Result<Sexp, String> {
    let mut r = Reader { s, i: 0 };
    let v = r.form()?;
    Ok(v)
}

/// Read one form and return it with the unread rest.
pub fn read_prefix(s: &str) -> Result<(Sexp, &str), String> {
    let mut r = Reader { s, i: 0 };
    let v = r.form()?;
    Ok((v, &s[r.i..]))
}

struct Reader<'a> {
    s: &'a str,
    i: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<char> {
        self.s[self.i..].chars().next()
    }

    fn skip(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => self.i += c.len_utf8(),
                Some(';') => {
                    while let Some(c) = self.peek() {
                        self.i += c.len_utf8();
                        if c == '\n' {
                            break;
                        }
                    }
                }
                _ => return,
            }
        }
    }

    fn form(&mut self) -> Result<Sexp, String> {
        self.skip();
        let Some(c) = self.peek() else {
            return Err("end of input".into());
        };
        match c {
            '(' => {
                self.i += 1;
                let mut items = vec![];
                loop {
                    self.skip();
                    match self.peek() {
                        None => return Err("unbalanced (".into()),
                        Some(')') => {
                            self.i += 1;
                            return Ok(if items.is_empty() { Sexp::Nil } else { Sexp::List(items) });
                        }
                        Some('.')
                            if self.s[self.i + 1..]
                                .chars()
                                .next()
                                .is_some_and(|c| c.is_whitespace() || c == '(') =>
                        {
                            self.i += 1;
                            let tail = self.form()?;
                            self.skip();
                            if self.peek() != Some(')') {
                                return Err("bad dotted list".into());
                            }
                            self.i += 1;
                            return Ok(if tail.is_nil() {
                                Sexp::List(items)
                            } else {
                                Sexp::Dotted(items, Box::new(tail))
                            });
                        }
                        Some(_) => items.push(self.form()?),
                    }
                }
            }
            '[' => {
                self.i += 1;
                let mut items = vec![];
                loop {
                    self.skip();
                    match self.peek() {
                        None => return Err("unbalanced [".into()),
                        Some(']') => {
                            self.i += 1;
                            return Ok(Sexp::Vector(items));
                        }
                        Some(_) => items.push(self.form()?),
                    }
                }
            }
            ')' | ']' => Err(format!("unexpected {c}")),
            '\'' | '`' => {
                self.i += 1;
                let v = self.form()?;
                Ok(Sexp::List(vec![Sexp::Sym("quote".into()), v]))
            }
            ',' => {
                self.i += 1;
                let v = self.form()?;
                Ok(Sexp::List(vec![Sexp::Sym(",".into()), v]))
            }
            '#' if self.s[self.i..].starts_with("#'") => {
                self.i += 2;
                let v = self.form()?;
                Ok(Sexp::List(vec![Sexp::Sym("function".into()), v]))
            }
            '"' => {
                self.i += 1;
                let mut out = String::new();
                loop {
                    let Some(c) = self.peek() else {
                        return Err("unterminated string".into());
                    };
                    self.i += c.len_utf8();
                    match c {
                        '"' => return Ok(Sexp::Str(out)),
                        '\\' => {
                            let Some(e) = self.peek() else {
                                return Err("unterminated string".into());
                            };
                            self.i += e.len_utf8();
                            match e {
                                'n' => out.push('\n'),
                                't' => out.push('\t'),
                                '\n' => {}
                                e => out.push(e),
                            }
                        }
                        c => out.push(c),
                    }
                }
            }
            '?' => {
                // A character literal: its code.
                self.i += 1;
                let Some(mut ch) = self.peek() else {
                    return Err("bad character".into());
                };
                self.i += ch.len_utf8();
                if ch == '\\'
                    && let Some(e) = self.peek()
                {
                    self.i += e.len_utf8();
                    ch = match e {
                        'n' => '\n',
                        't' => '\t',
                        e => e,
                    };
                }
                Ok(Sexp::Int(ch as i64))
            }
            _ => {
                let start = self.i;
                while let Some(c) = self.peek() {
                    if c.is_whitespace() || "()[]\";'".contains(c) {
                        break;
                    }
                    if c == '\\' {
                        self.i += 1;
                    }
                    self.i += self.peek().map_or(0, char::len_utf8);
                }
                let tok = &self.s[start..self.i];
                Ok(atom(tok))
            }
        }
    }
}

fn atom(tok: &str) -> Sexp {
    if tok == "nil" {
        return Sexp::Nil;
    }
    if tok == "t" {
        return Sexp::T;
    }
    if let Ok(i) = tok.parse::<i64>() {
        return Sexp::Int(i);
    }
    if tok.contains(['.', 'e']) && tok.chars().next().is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '.')
        && let Ok(f) = tok.parse::<f64>()
    {
        return Sexp::Float(f);
    }
    Sexp::Sym(tok.replace('\\', ""))
}

/// Strip a leading `(quote X)`.
pub fn unquote(s: Sexp) -> Sexp {
    match s {
        Sexp::List(mut v) if v.len() == 2 && v[0] == Sexp::Sym("quote".into()) => v.pop().unwrap(),
        s => s,
    }
}

/// An option as Lisp data: TOML mapped, or a Lisp form in a string.
pub fn option(name: &str) -> Option<Sexp> {
    super::options::get(name).map(|v| unquote(Sexp::from_toml(&v)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_lisp_data() {
        let v = read("((sequence \"TODO\" \"|\" \"DONE\") (type . 3) [1 2.5] ?a 'x)").unwrap();
        assert_eq!(
            v.to_string(),
            "((sequence \"TODO\" \"|\" \"DONE\") (type . 3) [1 2.5] 97 (quote x))"
        );
        assert_eq!(read("(a . b)").unwrap().cdr(), Sexp::Sym("b".into()));
        assert_eq!(read("(:a 1 :b 2)").unwrap().plist_get(":b"), Some(&Sexp::Int(2)));
        assert!(read("(a").is_err());
    }

    #[test]
    fn maps_toml() {
        let t: toml::Table = toml::from_str(
            "a = [[\"sequence\", \"TODO\"]]\nb = \"'((x . 1))\"\nc = [\"k\", \".\", 2]",
        )
        .unwrap();
        assert_eq!(Sexp::from_toml(&t["a"]).to_string(), "((\"sequence\" \"TODO\"))");
        assert_eq!(unquote(Sexp::from_toml(&t["b"])).to_string(), "((x . 1))");
        assert_eq!(Sexp::from_toml(&t["c"]).to_string(), "(\"k\" . 2)");
    }
}
