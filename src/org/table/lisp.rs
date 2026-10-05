//! An interpreter for the Emacs Lisp that table formulas (`'(...)`),
//! orgtbl translator parameters (`:fmt (lambda (c) ...)`) and plot
//! options use: numbers, strings, symbols, lists, lambdas; `let`, `if`,
//! `cond`, `and`/`or`, `mapcar`/`apply` and the common list, string and
//! arithmetic functions, plus `org-lookup-*` and `orgtbl-ascii-draw`.
//!
//! Lisp outside this subset falls back to `emacs --batch -Q` when Emacs
//! is installed (see [`eval_formula`]).

use std::fmt::Write as _;
use std::rc::Rc;

/// A Lisp number (Emacs `string-to-number` results).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Num {
    I(i64),
    F(f64),
}

impl Num {
    pub fn f(self) -> f64 {
        match self {
            Num::I(i) => i as f64,
            Num::F(f) => f,
        }
    }

    pub fn add(&self, o: &Num) -> Num {
        match (self, o) {
            (Num::I(a), Num::I(b)) => a.checked_add(*b).map_or(Num::F(*a as f64 + *b as f64), Num::I),
            _ => Num::F(self.f() + o.f()),
        }
    }

    pub fn sub(&self, o: &Num) -> Num {
        match (self, o) {
            (Num::I(a), Num::I(b)) => a.checked_sub(*b).map_or(Num::F(*a as f64 - *b as f64), Num::I),
            _ => Num::F(self.f() - o.f()),
        }
    }

    /// The sum of `items` parsed as numbers, as `apply '+` would type it.
    pub fn from_f64_sum(v: f64, items: &[String]) -> Num {
        let all_int = items.iter().all(|s| matches!(string_to_number(s), Num::I(_)) || !s.contains(['.', 'e', 'E', ':']));
        if all_int && v.fract() == 0.0 && v.abs() < 9e15 { Num::I(v as i64) } else { Num::F(v) }
    }
}

/// Emacs `string-to-number` (base 10).
pub fn string_to_number(s: &str) -> Num {
    let s = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0c']);
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
        i += 1;
    }
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let int_digits = i - int_start;
    let mut float = false;
    let mut end = i;
    if i < b.len() && b[i] == b'.' {
        let mut j = i + 1;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > i + 1 {
            float = true;
            end = j;
        } else if int_digits > 0 {
            end = i + 1;
        }
        i = j;
    }
    let digits_seen = int_digits > 0 || float;
    if digits_seen && i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'-' || b[j] == b'+') {
            j += 1;
        }
        let k = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > k {
            float = true;
            end = j;
        } else if j == k && b.get(k - 1).is_some_and(|c| *c == b'e' || *c == b'E') && b[k..].starts_with(b"+INF") {
            return Num::F(if b[0] == b'-' { f64::NEG_INFINITY } else { f64::INFINITY });
        }
    }
    if !digits_seen {
        return Num::I(0);
    }
    let text = &s[..end];
    if float {
        Num::F(text.trim_end_matches('.').parse().unwrap_or(0.0))
    } else {
        let t = text.trim_end_matches('.');
        t.parse::<i64>().map(Num::I).unwrap_or_else(|_| Num::F(t.parse().unwrap_or(0.0)))
    }
}

/// `%.Ng` formatting.
pub fn fmt_g(x: f64, prec: usize, alt: bool) -> String {
    if x == 0.0 {
        return if x.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    if !x.is_finite() {
        return float_special(x);
    }
    let prec = prec.max(1);
    let e = format!("{:.*e}", prec - 1, x);
    let (mant, exp) = e.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    if exp < -4 || exp >= prec as i32 {
        let mut m = mant.to_owned();
        if !alt && m.contains('.') {
            m = m.trim_end_matches('0').trim_end_matches('.').to_owned();
        }
        format!("{}e{}{:02}", m, if exp < 0 { '-' } else { '+' }, exp.abs())
    } else {
        let decimals = (prec as i32 - 1 - exp).max(0) as usize;
        let mut f = format!("{x:.decimals$}");
        if !alt && f.contains('.') {
            f = f.trim_end_matches('0').trim_end_matches('.').to_owned();
        }
        f
    }
}

fn float_special(x: f64) -> String {
    if x.is_nan() {
        if x.is_sign_negative() { "-0.0e+NaN".into() } else { "0.0e+NaN".into() }
    } else if x > 0.0 {
        "1.0e+INF".into()
    } else {
        "-1.0e+INF".into()
    }
}

/// How Emacs prints a float (`number-to-string`, `prin1`).
pub fn float_to_string(x: f64) -> String {
    if !x.is_finite() {
        return float_special(x);
    }
    let mut s = String::new();
    for p in 15..=17 {
        s = fmt_g(x, p, false);
        if s.parse::<f64>().ok() == Some(x) {
            break;
        }
    }
    if !s.contains(['.', 'e']) {
        s.push_str(".0");
    }
    s
}

pub fn num_to_string(n: &Num) -> String {
    match n {
        Num::I(i) => i.to_string(),
        Num::F(f) => float_to_string(*f),
    }
}

// ---- values ----

/// A Lisp object.
#[derive(Clone, Debug)]
pub enum L {
    Nil,
    T,
    Int(i64),
    Float(f64),
    Str(Rc<str>),
    Sym(Rc<str>),
    List(Rc<Vec<L>>),
    /// `(a b . c)`.
    Dotted(Rc<Vec<L>>, Rc<L>),
    Vector(Rc<Vec<L>>),
    Lambda(Rc<Lambda>),
}

#[derive(Debug)]
pub struct Lambda {
    params: Vec<String>,
    body: Vec<L>,
    env: Vec<(Rc<str>, L)>,
}

impl PartialEq for L {
    fn eq(&self, o: &L) -> bool {
        equal(self, o)
    }
}

pub fn sym(s: &str) -> L {
    match s {
        "nil" => L::Nil,
        "t" => L::T,
        _ => L::Sym(s.into()),
    }
}

pub fn str_(s: &str) -> L {
    L::Str(s.into())
}

pub fn list(v: Vec<L>) -> L {
    if v.is_empty() { L::Nil } else { L::List(Rc::new(v)) }
}

impl L {
    pub fn is_nil(&self) -> bool {
        matches!(self, L::Nil)
    }

    pub fn truthy(&self) -> bool {
        !self.is_nil()
    }

    /// A proper list's elements.
    pub fn items(&self) -> Result<Vec<L>, String> {
        match self {
            L::Nil => Ok(vec![]),
            L::List(v) => Ok(v.to_vec()),
            L::Vector(v) => Ok(v.to_vec()),
            other => Err(format!("Wrong type argument: listp, {}", prin1(other))),
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            L::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn sym_name(&self) -> Option<&str> {
        match self {
            L::Sym(s) => Some(s),
            L::Nil => Some("nil"),
            L::T => Some("t"),
            _ => None,
        }
    }

    fn num(&self) -> Result<Num, String> {
        match self {
            L::Int(i) => Ok(Num::I(*i)),
            L::Float(f) => Ok(Num::F(*f)),
            o => Err(format!("Wrong type argument: number-or-marker-p, {}", prin1(o))),
        }
    }

    fn int(&self) -> Result<i64, String> {
        match self {
            L::Int(i) => Ok(*i),
            o => Err(format!("Wrong type argument: integerp, {}", prin1(o))),
        }
    }

    fn string(&self) -> Result<Rc<str>, String> {
        match self {
            L::Str(s) => Ok(s.clone()),
            o => Err(format!("Wrong type argument: stringp, {}", prin1(o))),
        }
    }
}

fn from_num(n: Num) -> L {
    match n {
        Num::I(i) => L::Int(i),
        Num::F(f) => L::Float(f),
    }
}

pub fn equal(a: &L, b: &L) -> bool {
    match (a, b) {
        (L::Nil, L::Nil) | (L::T, L::T) => true,
        (L::Int(x), L::Int(y)) => x == y,
        (L::Float(x), L::Float(y)) => x == y || (x.is_nan() && y.is_nan()),
        (L::Str(x), L::Str(y)) | (L::Sym(x), L::Sym(y)) => x == y,
        (L::List(x), L::List(y)) | (L::Vector(x), L::Vector(y)) => x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| equal(p, q)),
        (L::Dotted(x, xt), L::Dotted(y, yt)) => x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| equal(p, q)) && equal(xt, yt),
        (L::Lambda(x), L::Lambda(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

/// `eq`: identity. Strings are distinct objects unless the same one (or
/// both empty, which Emacs shares).
fn eq(a: &L, b: &L) -> bool {
    match (a, b) {
        (L::Str(x), L::Str(y)) => Rc::ptr_eq(x, y) || (x.is_empty() && y.is_empty()),
        (L::Float(x), L::Float(y)) => x.to_bits() == y.to_bits(),
        (L::List(x), L::List(y)) => Rc::ptr_eq(x, y),
        (L::Vector(x), L::Vector(y)) => Rc::ptr_eq(x, y),
        (L::Dotted(x, _), L::Dotted(y, _)) => Rc::ptr_eq(x, y),
        _ => equal(a, b),
    }
}

fn eql(a: &L, b: &L) -> bool {
    match (a, b) {
        (L::Float(x), L::Float(y)) => x.to_bits() == y.to_bits(),
        _ => eq(a, b),
    }
}

// ---- printing ----

fn escape_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn print(x: &L, quote: bool) -> String {
    match x {
        L::Nil => "nil".into(),
        L::T => "t".into(),
        L::Int(i) => i.to_string(),
        L::Float(f) => float_to_string(*f),
        L::Str(s) => {
            if quote {
                escape_str(s)
            } else {
                s.to_string()
            }
        }
        L::Sym(s) => s.to_string(),
        L::List(v) => {
            if v.len() == 2 && quote_sym(&v[0]) == Some("quote") {
                return format!("'{}", print(&v[1], quote));
            }
            if v.len() == 2 && quote_sym(&v[0]) == Some("function") {
                return format!("#'{}", print(&v[1], quote));
            }
            format!("({})", v.iter().map(|e| print(e, quote)).collect::<Vec<_>>().join(" "))
        }
        L::Dotted(v, t) => format!("({} . {})", v.iter().map(|e| print(e, quote)).collect::<Vec<_>>().join(" "), print(t, quote)),
        L::Vector(v) => format!("[{}]", v.iter().map(|e| print(e, quote)).collect::<Vec<_>>().join(" ")),
        L::Lambda(l) => format!(
            "#[({}) {} nil]",
            l.params.join(" "),
            l.body.iter().map(|e| print(e, true)).collect::<Vec<_>>().join(" ")
        ),
    }
}

fn quote_sym(x: &L) -> Option<&str> {
    match x {
        L::Sym(s) => Some(s),
        _ => None,
    }
}

/// `prin1-to-string`.
pub fn prin1(x: &L) -> String {
    print(x, true)
}

/// `princ` / `format "%s"`.
pub fn princ(x: &L) -> String {
    print(x, false)
}

// ---- reader ----

struct Reader<'a> {
    s: &'a [u8],
    src: &'a str,
    i: usize,
}

const SYM_CHARS: &str = "-+*/<>=!?$%&:^_~.@";

impl Reader<'_> {
    fn skip(&mut self) {
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c.is_ascii_whitespace() {
                self.i += 1;
            } else if c == b';' {
                while self.i < self.s.len() && self.s[self.i] != b'\n' {
                    self.i += 1;
                }
            } else {
                break;
            }
        }
    }

    fn read(&mut self) -> Result<L, String> {
        self.skip();
        let Some(&c) = self.s.get(self.i) else {
            return Err("End of file during parsing".into());
        };
        match c {
            b'(' => {
                self.i += 1;
                let mut v = vec![];
                loop {
                    self.skip();
                    match self.s.get(self.i) {
                        None => return Err("End of file during parsing".into()),
                        Some(b')') => {
                            self.i += 1;
                            return Ok(list(v));
                        }
                        Some(b'.') if self.s.get(self.i + 1).is_none_or(|c| c.is_ascii_whitespace() || *c == b'(' || *c == b')') => {
                            self.i += 1;
                            let tail = self.read()?;
                            self.skip();
                            if self.s.get(self.i) != Some(&b')') {
                                return Err("Invalid read syntax: . in wrong context".into());
                            }
                            self.i += 1;
                            return Ok(match tail {
                                L::Nil => list(v),
                                L::List(t) => {
                                    v.extend(t.iter().cloned());
                                    list(v)
                                }
                                t => L::Dotted(Rc::new(v), Rc::new(t)),
                            });
                        }
                        _ => v.push(self.read()?),
                    }
                }
            }
            b'[' => {
                self.i += 1;
                let mut v = vec![];
                loop {
                    self.skip();
                    match self.s.get(self.i) {
                        None => return Err("End of file during parsing".into()),
                        Some(b']') => {
                            self.i += 1;
                            return Ok(L::Vector(Rc::new(v)));
                        }
                        _ => v.push(self.read()?),
                    }
                }
            }
            b')' | b']' => {
                self.i += 1;
                Err("Invalid read syntax: )".into())
            }
            b'\'' => {
                self.i += 1;
                Ok(list(vec![sym("quote"), self.read()?]))
            }
            b'`' => {
                self.i += 1;
                Ok(list(vec![sym("`"), self.read()?]))
            }
            b',' => {
                self.i += 1;
                if self.s.get(self.i) == Some(&b'@') {
                    self.i += 1;
                    Ok(list(vec![sym(",@"), self.read()?]))
                } else {
                    Ok(list(vec![sym(","), self.read()?]))
                }
            }
            b'#' if self.s.get(self.i + 1) == Some(&b'\'') => {
                self.i += 2;
                Ok(list(vec![sym("function"), self.read()?]))
            }
            b'"' => {
                self.i += 1;
                let mut out = String::new();
                let rest = &self.src[self.i..];
                let mut chars = rest.char_indices();
                while let Some((j, ch)) = chars.next() {
                    match ch {
                        '"' => {
                            self.i += j + 1;
                            return Ok(L::Str(out.into()));
                        }
                        '\\' => match chars.next() {
                            Some((_, 'n')) => out.push('\n'),
                            Some((_, 't')) => out.push('\t'),
                            Some((_, 'r')) => out.push('\r'),
                            Some((_, 'e')) => out.push('\x1b'),
                            Some((_, 'a')) => out.push('\x07'),
                            Some((_, 'f')) => out.push('\x0c'),
                            Some((_, '\n')) => {}
                            Some((_, ' ')) => {}
                            Some((_, c)) => out.push(c),
                            None => break,
                        },
                        c => out.push(c),
                    }
                }
                Err("End of file during parsing".into())
            }
            b'?' => {
                self.i += 1;
                let rest = &self.src[self.i..];
                let mut it = rest.chars();
                let ch = it.next().ok_or("End of file during parsing")?;
                if ch == '\\' {
                    let e = it.next().ok_or("End of file during parsing")?;
                    self.i += 1 + e.len_utf8();
                    let v = match e {
                        'n' => '\n',
                        't' => '\t',
                        's' => ' ',
                        'r' => '\r',
                        'e' => '\x1b',
                        c => c,
                    };
                    Ok(L::Int(v as i64))
                } else {
                    self.i += ch.len_utf8();
                    Ok(L::Int(ch as i64))
                }
            }
            _ => {
                let start = self.i;
                let mut tok = String::new();
                while self.i < self.s.len() {
                    let ch = self.src[self.i..].chars().next().unwrap();
                    if ch == '\\' {
                        if let Some(n) = self.src[self.i + 1..].chars().next() {
                            tok.push(n);
                            self.i += 1 + n.len_utf8();
                            continue;
                        }
                    }
                    if ch.is_alphanumeric() || SYM_CHARS.contains(ch) || (!ch.is_ascii() && !ch.is_whitespace()) {
                        tok.push(ch);
                        self.i += ch.len_utf8();
                    } else {
                        break;
                    }
                }
                if self.i == start {
                    self.i += 1;
                    return Err(format!("Invalid read syntax: {}", c as char));
                }
                Ok(atom(&tok))
            }
        }
    }
}

fn atom(tok: &str) -> L {
    let b = tok.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let digits = |from: usize| b[from..].iter().take_while(|c| c.is_ascii_digit()).count();
    let n1 = digits(i);
    let mut j = i + n1;
    // Integer: digits with an optional trailing dot.
    if n1 > 0 && (j == b.len() || (j + 1 == b.len() && b[j] == b'.')) {
        let t = tok.trim_end_matches('.');
        return t.parse::<i64>().map(L::Int).unwrap_or_else(|_| L::Float(t.parse().unwrap_or(0.0)));
    }
    // Float: digits? . digits (e[+-]digits)? or digits e exp.
    let mut frac = 0;
    if j < b.len() && b[j] == b'.' {
        frac = digits(j + 1);
        j += 1 + frac;
    }
    if n1 + frac > 0 {
        let mut ok = frac > 0 || n1 > 0;
        let mut exp = false;
        if j < b.len() && (b[j] == b'e' || b[j] == b'E') {
            let mut k = j + 1;
            if k < b.len() && (b[k] == b'+' || b[k] == b'-') {
                k += 1;
            }
            let e = digits(k);
            if e > 0 {
                j = k + e;
                exp = true;
            } else if tok[k..].eq_ignore_ascii_case("INF") {
                return L::Float(if b[0] == b'-' { f64::NEG_INFINITY } else { f64::INFINITY });
            } else if tok[k..].eq_ignore_ascii_case("NaN") {
                return L::Float(f64::NAN);
            } else {
                ok = false;
            }
        }
        if ok && j == b.len() && (frac > 0 || exp) {
            return L::Float(tok.parse().unwrap_or(0.0));
        }
    }
    sym(tok)
}

/// Read one object from `s`.
pub fn read(s: &str) -> Result<L, String> {
    let mut r = Reader { s: s.as_bytes(), src: s, i: 0 };
    r.read()
}

/// Read every object in `s` (a parameter list without its parentheses).
pub fn read_all(s: &str) -> Result<Vec<L>, String> {
    let mut r = Reader { s: s.as_bytes(), src: s, i: 0 };
    let mut out = vec![];
    loop {
        r.skip();
        if r.i >= r.s.len() {
            return Ok(out);
        }
        out.push(r.read()?);
    }
}

// ---- evaluation ----

/// The interpreter: variable bindings and a step budget.
pub struct Interp {
    env: Vec<(Rc<str>, L)>,
    steps: usize,
    /// Functions outside the subset were called.
    pub unknown: Option<String>,
}

impl Default for Interp {
    fn default() -> Self {
        Interp { env: vec![], steps: 0, unknown: None }
    }
}

type E = Result<L, String>;

fn arith_err() -> String {
    "Arithmetic error".into()
}

impl Interp {
    pub fn new() -> Interp {
        Interp::default()
    }

    fn lookup(&self, name: &str) -> E {
        if let Some((_, v)) = self.env.iter().rev().find(|(k, _)| &**k == name) {
            return Ok(v.clone());
        }
        if name.starts_with(':') {
            return Ok(sym(name));
        }
        match name {
            "most-positive-fixnum" => Ok(L::Int(i64::MAX >> 2)),
            "most-negative-fixnum" => Ok(L::Int(i64::MIN >> 2)),
            "float-pi" | "pi" => Ok(L::Float(std::f64::consts::PI)),
            "float-e" => Ok(L::Float(std::f64::consts::E)),
            _ => {
                if let Some(v) = super::super::sexp::option(name) {
                    return Ok(from_sexp(&v));
                }
                Err(format!("Symbol's value as variable is void: {name}"))
            }
        }
    }

    fn set(&mut self, name: &str, v: L) {
        if let Some(slot) = self.env.iter_mut().rev().find(|(k, _)| &**k == name) {
            slot.1 = v;
        } else {
            self.env.insert(0, (name.into(), v));
        }
    }

    pub fn eval(&mut self, x: &L) -> E {
        self.steps += 1;
        if self.steps > 2_000_000 {
            return Err("Lisp evaluation step limit exceeded".into());
        }
        match x {
            L::Sym(s) => self.lookup(s),
            L::List(v) => self.eval_form(v),
            L::Dotted(..) => Err("Invalid function".into()),
            other => Ok(other.clone()),
        }
    }

    fn progn(&mut self, body: &[L]) -> E {
        let mut r = L::Nil;
        for f in body {
            r = self.eval(f)?;
        }
        Ok(r)
    }

    fn eval_form(&mut self, v: &[L]) -> E {
        let head = &v[0];
        let args = &v[1..];
        if let L::List(lv) = head
            && lv.first().and_then(L::sym_name) == Some("lambda")
        {
            let f = self.make_lambda(&lv[1..])?;
            let vals = args.iter().map(|a| self.eval(a)).collect::<Result<Vec<_>, _>>()?;
            return self.apply(&f, vals);
        }
        let Some(name) = head.sym_name() else {
            return Err(format!("Invalid function: {}", prin1(head)));
        };
        let name = name.to_owned();
        match name.as_str() {
            "quote" => return args.first().cloned().ok_or_else(|| "Wrong number of arguments: quote".to_string()),
            "function" => {
                return match args.first() {
                    Some(L::List(l)) if l.first().and_then(L::sym_name) == Some("lambda") => self.make_lambda(&l[1..]),
                    Some(x) => Ok(x.clone()),
                    None => Ok(L::Nil),
                };
            }
            "lambda" => return self.make_lambda(args),
            "`" => return self.backquote(args.first().unwrap_or(&L::Nil)),
            "if" => {
                let c = self.eval(args.first().unwrap_or(&L::Nil))?;
                return if c.truthy() {
                    self.eval(args.get(1).unwrap_or(&L::Nil))
                } else {
                    self.progn(args.get(2..).unwrap_or(&[]))
                };
            }
            "when" => {
                let c = self.eval(args.first().unwrap_or(&L::Nil))?;
                return if c.truthy() { self.progn(&args[1..]) } else { Ok(L::Nil) };
            }
            "unless" => {
                let c = self.eval(args.first().unwrap_or(&L::Nil))?;
                return if c.truthy() { Ok(L::Nil) } else { self.progn(&args[1..]) };
            }
            "cond" => {
                for clause in args {
                    let c = clause.items()?;
                    let Some(test) = c.first() else { continue };
                    let v = self.eval(test)?;
                    if v.truthy() {
                        return if c.len() > 1 { self.progn(&c[1..]) } else { Ok(v) };
                    }
                }
                return Ok(L::Nil);
            }
            "and" => {
                let mut r = L::T;
                for a in args {
                    r = self.eval(a)?;
                    if r.is_nil() {
                        return Ok(L::Nil);
                    }
                }
                return Ok(r);
            }
            "or" => {
                for a in args {
                    let r = self.eval(a)?;
                    if r.truthy() {
                        return Ok(r);
                    }
                }
                return Ok(L::Nil);
            }
            "progn" => return self.progn(args),
            "prog1" => {
                let r = self.eval(args.first().unwrap_or(&L::Nil))?;
                self.progn(&args[1..])?;
                return Ok(r);
            }
            "prog2" => {
                self.eval(args.first().unwrap_or(&L::Nil))?;
                let r = self.eval(args.get(1).unwrap_or(&L::Nil))?;
                self.progn(args.get(2..).unwrap_or(&[]))?;
                return Ok(r);
            }
            "let" | "let*" => {
                let star = name == "let*";
                let binds = args.first().unwrap_or(&L::Nil).items()?;
                let mark = self.env.len();
                let mut pending = vec![];
                for b in binds {
                    let (k, v) = match &b {
                        L::Sym(s) => (s.clone(), L::Nil),
                        L::List(p) => {
                            let k = p[0].sym_name().ok_or("Wrong type argument: symbolp")?.into();
                            let v = match p.get(1) {
                                Some(e) => self.eval(e)?,
                                None => L::Nil,
                            };
                            (k, v)
                        }
                        _ => return Err("Wrong type argument: symbolp".into()),
                    };
                    if star {
                        self.env.push((k, v));
                    } else {
                        pending.push((k, v));
                    }
                }
                self.env.extend(pending);
                let r = self.progn(&args[1..]);
                self.env.truncate(mark);
                return r;
            }
            "setq" | "setf" => {
                let mut r = L::Nil;
                for pair in args.chunks(2) {
                    let k = pair[0].sym_name().ok_or("Wrong type argument: symbolp")?.to_owned();
                    r = self.eval(pair.get(1).unwrap_or(&L::Nil))?;
                    self.set(&k, r.clone());
                }
                return Ok(r);
            }
            "while" => {
                while self.eval(args.first().unwrap_or(&L::Nil))?.truthy() {
                    self.progn(&args[1..])?;
                }
                return Ok(L::Nil);
            }
            "dolist" | "dotimes" => {
                let spec = args.first().unwrap_or(&L::Nil).items()?;
                let var: Rc<str> = spec.first().and_then(L::sym_name).ok_or("Wrong type argument: symbolp")?.into();
                let seq = self.eval(spec.get(1).unwrap_or(&L::Nil))?;
                let items: Vec<L> = if name == "dolist" {
                    seq.items()?
                } else {
                    (0..seq.int()?).map(L::Int).collect()
                };
                let mark = self.env.len();
                for it in items {
                    self.env.push((var.clone(), it));
                    let r = self.progn(&args[1..]);
                    self.env.truncate(mark);
                    r?;
                }
                return match spec.get(2) {
                    Some(res) => self.eval(res),
                    None => Ok(L::Nil),
                };
            }
            "ignore-errors" => return Ok(self.progn(args).unwrap_or(L::Nil)),
            "condition-case" => {
                let var = args.first().and_then(L::sym_name).unwrap_or("nil").to_owned();
                return match self.eval(args.get(1).unwrap_or(&L::Nil)) {
                    Ok(v) => Ok(v),
                    Err(e) => {
                        for h in args.get(2..).unwrap_or(&[]) {
                            let h = h.items()?;
                            let mark = self.env.len();
                            if var != "nil" {
                                self.env.push((var.as_str().into(), list(vec![sym("error"), str_(&e)])));
                            }
                            let r = self.progn(&h[1..]);
                            self.env.truncate(mark);
                            return r;
                        }
                        Err(e)
                    }
                };
            }
            "pcase" | "cl-case" | "pcase-let" | "defun" | "defvar" | "defmacro" => {
                self.unknown = Some(name.clone());
                return Err(format!("Unsupported form: {name}"));
            }
            _ => {}
        }
        let vals = args.iter().map(|a| self.eval(a)).collect::<Result<Vec<_>, _>>()?;
        self.call(&name, vals)
    }

    fn backquote(&mut self, x: &L) -> E {
        match x {
            L::List(v) => {
                if v.first().and_then(L::sym_name) == Some(",") {
                    return self.eval(v.get(1).unwrap_or(&L::Nil));
                }
                let mut out = vec![];
                for e in v.iter() {
                    if let L::List(ev) = e
                        && ev.first().and_then(L::sym_name) == Some(",@")
                    {
                        out.extend(self.eval(ev.get(1).unwrap_or(&L::Nil))?.items()?);
                        continue;
                    }
                    out.push(self.backquote(e)?);
                }
                Ok(list(out))
            }
            other => Ok(other.clone()),
        }
    }

    fn make_lambda(&mut self, rest: &[L]) -> E {
        let params = rest
            .first()
            .unwrap_or(&L::Nil)
            .items()?
            .iter()
            .map(|p| p.sym_name().map(str::to_owned).ok_or("Invalid lambda list".to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(L::Lambda(Rc::new(Lambda { params, body: rest[1..].to_vec(), env: self.env.clone() })))
    }

    /// Call a function value (symbol or lambda).
    pub fn apply(&mut self, f: &L, args: Vec<L>) -> E {
        match f {
            L::Lambda(l) => {
                let saved = std::mem::replace(&mut self.env, l.env.clone());
                let mut it = args.into_iter();
                let mut optional = false;
                let mut i = 0;
                while i < l.params.len() {
                    let p = &l.params[i];
                    match p.as_str() {
                        "&optional" => optional = true,
                        "&rest" => {
                            if let Some(r) = l.params.get(i + 1) {
                                self.env.push((r.as_str().into(), list(it.by_ref().collect())));
                            }
                            break;
                        }
                        _ => match it.next() {
                            Some(v) => self.env.push((p.as_str().into(), v)),
                            None if optional => self.env.push((p.as_str().into(), L::Nil)),
                            None => {
                                self.env = saved;
                                return Err("Wrong number of arguments".into());
                            }
                        },
                    }
                    i += 1;
                }
                let r = self.progn(&l.body);
                self.env = saved;
                r
            }
            L::List(v) if v.first().and_then(L::sym_name) == Some("lambda") => {
                let lam = self.make_lambda(&v[1..])?;
                self.apply(&lam, args)
            }
            L::Sym(s) => {
                let s = s.to_string();
                self.call(&s, args)
            }
            other => Err(format!("Invalid function: {}", prin1(other))),
        }
    }

    fn num_args(&self, args: &[L]) -> Result<Vec<Num>, String> {
        args.iter().map(L::num).collect()
    }

    fn compare(&self, args: &[L], f: impl Fn(f64, f64) -> bool) -> E {
        let n = self.num_args(args)?;
        Ok(if n.windows(2).all(|w| f(w[0].f(), w[1].f())) { L::T } else { L::Nil })
    }

    /// Call builtin `name`.
    pub fn call(&mut self, name: &str, a: Vec<L>) -> E {
        let arg = |i: usize| a.get(i).cloned().unwrap_or(L::Nil);
        let b = |v: bool| if v { L::T } else { L::Nil };
        Ok(match name {
            "+" | "*" => {
                let plus = name == "+";
                let mut acc = Num::I(if plus { 0 } else { 1 });
                for n in self.num_args(&a)? {
                    acc = match (acc, n) {
                        (Num::I(x), Num::I(y)) => {
                            let r = if plus { x.checked_add(y) } else { x.checked_mul(y) };
                            r.map_or_else(|| Num::F(if plus { x as f64 + y as f64 } else { x as f64 * y as f64 }), Num::I)
                        }
                        (x, y) => Num::F(if plus { x.f() + y.f() } else { x.f() * y.f() }),
                    };
                }
                from_num(acc)
            }
            "-" => {
                let n = self.num_args(&a)?;
                match n.len() {
                    0 => L::Int(0),
                    1 => from_num(match n[0] {
                        Num::I(i) => Num::I(-i),
                        Num::F(f) => Num::F(-f),
                    }),
                    _ => {
                        let mut acc = n[0];
                        for x in &n[1..] {
                            acc = acc.sub(x);
                        }
                        from_num(acc)
                    }
                }
            }
            "/" => {
                let n = self.num_args(&a)?;
                if n.is_empty() {
                    return Err("Wrong number of arguments: /".into());
                }
                let float = n.iter().any(|x| matches!(x, Num::F(_)));
                if n.len() == 1 {
                    return Ok(if float { L::Float(1.0 / n[0].f()) } else if n[0] == Num::I(0) { return Err(arith_err()) } else { L::Int(1 / n[0].f() as i64) });
                }
                if float {
                    let mut acc = n[0].f();
                    for x in &n[1..] {
                        acc /= x.f();
                    }
                    L::Float(acc)
                } else {
                    let mut acc = match n[0] {
                        Num::I(i) => i,
                        _ => 0,
                    };
                    for x in &n[1..] {
                        let Num::I(d) = *x else { unreachable!() };
                        if d == 0 {
                            return Err(arith_err());
                        }
                        acc /= d;
                    }
                    L::Int(acc)
                }
            }
            "%" => {
                let (x, y) = (arg(0).int()?, arg(1).int()?);
                if y == 0 {
                    return Err(arith_err());
                }
                L::Int(x % y)
            }
            "mod" | "cl-mod" => match (arg(0).num()?, arg(1).num()?) {
                (Num::I(x), Num::I(y)) => {
                    if y == 0 {
                        return Err(arith_err());
                    }
                    let r = x % y;
                    L::Int(if r != 0 && (r < 0) != (y < 0) { r + y } else { r })
                }
                (x, y) => {
                    let r = x.f() % y.f();
                    L::Float(if r != 0.0 && (r < 0.0) != (y.f() < 0.0) { r + y.f() } else { r })
                }
            },
            "1+" => from_num(arg(0).num()?.add(&Num::I(1))),
            "1-" => from_num(arg(0).num()?.sub(&Num::I(1))),
            "abs" => from_num(match arg(0).num()? {
                Num::I(i) => Num::I(i.abs()),
                Num::F(f) => Num::F(f.abs()),
            }),
            "max" | "min" => {
                let n = self.num_args(&a)?;
                let float = n.iter().any(|x| matches!(x, Num::F(_)));
                let mut best = *n.first().ok_or("Wrong number of arguments")?;
                for x in &n[1..] {
                    if (name == "max" && x.f() > best.f()) || (name == "min" && x.f() < best.f()) {
                        best = *x;
                    }
                }
                if float { L::Float(best.f()) } else { from_num(best) }
            }
            "float" => L::Float(arg(0).num()?.f()),
            "truncate" | "floor" | "ceiling" | "round" => {
                let mut x = arg(0).num()?;
                if let Some(d) = a.get(1).filter(|d| !d.is_nil()) {
                    let d = d.num()?;
                    x = match (x, d) {
                        (Num::I(p), Num::I(q)) if name == "floor" => {
                            if q == 0 {
                                return Err(arith_err());
                            }
                            return Ok(L::Int(p.div_euclid(q) - if q < 0 && p.rem_euclid(q) != 0 { 1 } else { 0 }));
                        }
                        _ => Num::F(x.f() / d.f()),
                    };
                }
                match x {
                    Num::I(i) => L::Int(i),
                    Num::F(f) => {
                        if !f.is_finite() {
                            return Err(arith_err());
                        }
                        L::Int(match name {
                            "truncate" => f.trunc(),
                            "floor" => f.floor(),
                            "ceiling" => f.ceil(),
                            _ => round_half_even(f),
                        } as i64)
                    }
                }
            }
            "ffloor" => L::Float(arg(0).num()?.f().floor()),
            "fceiling" => L::Float(arg(0).num()?.f().ceil()),
            "fround" => L::Float(round_half_even(arg(0).num()?.f())),
            "ftruncate" => L::Float(arg(0).num()?.f().trunc()),
            "expt" => match (arg(0).num()?, arg(1).num()?) {
                (Num::I(x), Num::I(y)) if y >= 0 => x.checked_pow(y as u32).map_or(L::Float((x as f64).powf(y as f64)), L::Int),
                (x, y) => L::Float(x.f().powf(y.f())),
            },
            "sqrt" => L::Float(arg(0).num()?.f().sqrt()),
            "exp" => L::Float(arg(0).num()?.f().exp()),
            "log" => {
                let x = arg(0).num()?.f();
                match a.get(1) {
                    Some(base) if !base.is_nil() => L::Float(x.ln() / base.num()?.f().ln()),
                    _ => L::Float(x.ln()),
                }
            }
            "sin" => L::Float(arg(0).num()?.f().sin()),
            "cos" => L::Float(arg(0).num()?.f().cos()),
            "tan" => L::Float(arg(0).num()?.f().tan()),
            "asin" => L::Float(arg(0).num()?.f().asin()),
            "acos" => L::Float(arg(0).num()?.f().acos()),
            "atan" => match a.get(1) {
                Some(x) if !x.is_nil() => L::Float(arg(0).num()?.f().atan2(x.num()?.f())),
                _ => L::Float(arg(0).num()?.f().atan()),
            },
            "isnan" => b(arg(0).num()?.f().is_nan()),
            "zerop" => b(arg(0).num()?.f() == 0.0),
            "natnump" => b(matches!(arg(0), L::Int(i) if i >= 0)),
            "cl-plusp" | "plusp" => b(arg(0).num()?.f() > 0.0),
            "cl-minusp" | "minusp" => b(arg(0).num()?.f() < 0.0),
            "cl-evenp" | "evenp" => b(arg(0).int()? % 2 == 0),
            "cl-oddp" | "oddp" => b(arg(0).int()? % 2 != 0),
            "=" => self.compare(&a, |x, y| x == y)?,
            "<" => self.compare(&a, |x, y| x < y)?,
            ">" => self.compare(&a, |x, y| x > y)?,
            "<=" => self.compare(&a, |x, y| x <= y)?,
            ">=" => self.compare(&a, |x, y| x >= y)?,
            "/=" => self.compare(&a, |x, y| x != y)?,
            "eq" => b(eq(&arg(0), &arg(1))),
            "eql" => b(eql(&arg(0), &arg(1))),
            "equal" => b(equal(&arg(0), &arg(1))),
            "string=" | "string-equal" => b(string_designator(&arg(0))? == string_designator(&arg(1))?),
            "string<" | "string-lessp" => b(string_designator(&arg(0))? < string_designator(&arg(1))?),
            "string>" | "string-greaterp" => b(string_designator(&arg(0))? > string_designator(&arg(1))?),
            "string-prefix-p" => {
                let (p, s) = (arg(0).string()?, arg(1).string()?);
                b(if arg(2).truthy() { s.to_lowercase().starts_with(&p.to_lowercase()) } else { s.starts_with(&*p) })
            }
            "string-suffix-p" => {
                let (p, s) = (arg(0).string()?, arg(1).string()?);
                b(if arg(2).truthy() { s.to_lowercase().ends_with(&p.to_lowercase()) } else { s.ends_with(&*p) })
            }
            "null" | "not" => b(arg(0).is_nil()),
            "numberp" | "number-or-marker-p" => b(matches!(arg(0), L::Int(_) | L::Float(_))),
            "integerp" | "fixnump" | "natnum" => b(matches!(arg(0), L::Int(_))),
            "floatp" => b(matches!(arg(0), L::Float(_))),
            "stringp" => b(matches!(arg(0), L::Str(_))),
            "symbolp" => b(matches!(arg(0), L::Sym(_) | L::Nil | L::T)),
            "keywordp" => b(matches!(&arg(0), L::Sym(s) if s.starts_with(':'))),
            "listp" => b(matches!(arg(0), L::Nil | L::List(_) | L::Dotted(..))),
            "consp" => b(matches!(arg(0), L::List(_) | L::Dotted(..))),
            "atom" => b(!matches!(arg(0), L::List(_) | L::Dotted(..))),
            "vectorp" => b(matches!(arg(0), L::Vector(_))),
            "booleanp" => b(matches!(arg(0), L::Nil | L::T)),
            "functionp" => b(match &arg(0) {
                L::Lambda(_) => true,
                L::Sym(s) => is_builtin(s),
                L::List(v) => v.first().and_then(L::sym_name) == Some("lambda"),
                _ => false,
            }),
            "type-of" => sym(match arg(0) {
                L::Nil | L::T | L::Sym(_) => "symbol",
                L::Int(_) => "integer",
                L::Float(_) => "float",
                L::Str(_) => "string",
                L::List(_) | L::Dotted(..) => "cons",
                L::Vector(_) => "vector",
                L::Lambda(_) => "interpreted-function",
            }),
            "identity" => arg(0),
            "ignore" => L::Nil,
            "concat" => {
                let mut s = String::new();
                for x in &a {
                    match x {
                        L::Str(t) => s.push_str(t),
                        L::Nil => {}
                        L::List(v) | L::Vector(v) => {
                            for c in v.iter() {
                                s.push(char::from_u32(c.int()? as u32).unwrap_or('?'));
                            }
                        }
                        o => return Err(format!("Wrong type argument: sequencep, {}", prin1(o))),
                    }
                }
                str_(&s)
            }
            "substring" | "substring-no-properties" | "seq-subseq" | "cl-subseq" => {
                let x = arg(0);
                match &x {
                    L::Str(s) => {
                        let chars: Vec<char> = s.chars().collect();
                        let (from, to) = slice_bounds(chars.len(), &arg(1), &arg(2))?;
                        str_(&chars[from..to].iter().collect::<String>())
                    }
                    _ => {
                        let v = x.items()?;
                        let (from, to) = slice_bounds(v.len(), &arg(1), &arg(2))?;
                        list(v[from..to].to_vec())
                    }
                }
            }
            "string-to-number" => from_num(string_to_number(&arg(0).string()?)),
            "number-to-string" => str_(&num_to_string(&arg(0).num()?)),
            "string-to-char" => L::Int(arg(0).string()?.chars().next().map_or(0, |c| c as i64)),
            "char-to-string" | "string" => {
                let mut s = String::new();
                for x in &a {
                    s.push(char::from_u32(x.int()? as u32).unwrap_or('?'));
                }
                str_(&s)
            }
            "make-string" => {
                let n = arg(0).int()?.max(0) as usize;
                let c = char::from_u32(arg(1).int()? as u32).unwrap_or(' ');
                str_(&c.to_string().repeat(n))
            }
            "format" | "format-message" => str_(&format(&arg(0).string()?, &a[1..])?),
            "message" => {
                if a.first().is_some_and(L::truthy) {
                    str_(&format(&arg(0).string()?, &a[1..])?)
                } else {
                    L::Nil
                }
            }
            "upcase" => match arg(0) {
                L::Int(c) => L::Int(char::from_u32(c as u32).map_or(c, |c| c.to_uppercase().next().unwrap_or(c) as i64)),
                x => str_(&x.string()?.to_uppercase()),
            },
            "downcase" => match arg(0) {
                L::Int(c) => L::Int(char::from_u32(c as u32).map_or(c, |c| c.to_lowercase().next().unwrap_or(c) as i64)),
                x => str_(&x.string()?.to_lowercase()),
            },
            "capitalize" | "upcase-initials" => {
                let s = arg(0).string()?;
                let mut out = String::new();
                let mut start = true;
                for c in s.chars() {
                    if c.is_alphanumeric() {
                        if start {
                            out.extend(c.to_uppercase());
                        } else if name == "capitalize" {
                            out.extend(c.to_lowercase());
                        } else {
                            out.push(c);
                        }
                        start = false;
                    } else {
                        out.push(c);
                        start = true;
                    }
                }
                str_(&out)
            }
            "string-trim" | "org-trim" => str_(arg(0).string()?.trim()),
            "string-trim-left" => str_(arg(0).string()?.trim_start()),
            "string-trim-right" => str_(arg(0).string()?.trim_end()),
            "string-empty-p" => b(arg(0).string()?.is_empty()),
            "string-blank-p" | "org-string-nw-p" => {
                let s = arg(0).string()?;
                let blank = s.chars().all(char::is_whitespace);
                if name == "string-blank-p" { b(blank) } else if blank { L::Nil } else { L::Str(s) }
            }
            "string-search" => {
                let (n, h) = (arg(0).string()?, arg(1).string()?);
                let start = if arg(2).is_nil() { 0 } else { arg(2).int()? as usize };
                let hc: String = h.chars().skip(start).collect();
                match hc.find(&*n) {
                    Some(i) => L::Int((start + hc[..i].chars().count()) as i64),
                    None => L::Nil,
                }
            }
            "string-replace" => str_(&arg(2).string()?.replace(&*arg(0).string()?, &arg(1).string()?)),
            "string-join" => {
                let parts = arg(0).items()?.iter().map(|x| x.string().map(|s| s.to_string())).collect::<Result<Vec<_>, _>>()?;
                let sep = if arg(1).is_nil() { Rc::from("") } else { arg(1).string()? };
                str_(&parts.join(&sep))
            }
            "split-string" => {
                let s = arg(0).string()?;
                let parts: Vec<L> = if arg(1).is_nil() {
                    s.split_whitespace().map(str_).collect()
                } else {
                    let r = super::emacs_regex(&arg(1).string()?)?;
                    let omit = arg(2).truthy();
                    r.split(&s).filter(|p| !omit || !p.is_empty()).map(str_).collect()
                };
                list(parts)
            }
            "string-match" | "string-match-p" => {
                let r = super::emacs_regex(&arg(0).string()?)?;
                let s = arg(1).string()?;
                match r.find(&s) {
                    Some(m) => L::Int(s[..m.start()].chars().count() as i64),
                    None => L::Nil,
                }
            }
            "replace-regexp-in-string" => {
                let r = super::emacs_regex(&arg(0).string()?)?;
                let rep = arg(1);
                let s = arg(2).string()?;
                match rep {
                    L::Str(t) => {
                        let lit = arg(4).truthy();
                        let t = if lit { t.replace('$', "$$") } else { emacs_replacement(&t) };
                        str_(&r.replace_all(&s, t.as_str()))
                    }
                    f => {
                        let mut out = String::new();
                        let mut last = 0;
                        for m in r.find_iter(&s) {
                            out.push_str(&s[last..m.start()]);
                            let v = self.apply(&f, vec![str_(m.as_str())])?;
                            out.push_str(&v.string()?);
                            last = m.end();
                        }
                        out.push_str(&s[last..]);
                        str_(&out)
                    }
                }
            }
            "symbol-name" => str_(arg(0).sym_name().ok_or("Wrong type argument: symbolp")?),
            "intern" | "intern-soft" => sym(&arg(0).string()?),
            "prin1-to-string" => str_(&if arg(1).truthy() { princ(&arg(0)) } else { prin1(&arg(0)) }),
            "list" => list(a),
            "vector" => L::Vector(Rc::new(a)),
            "cons" => match arg(1) {
                L::Nil => list(vec![arg(0)]),
                L::List(v) => {
                    let mut n = vec![arg(0)];
                    n.extend(v.iter().cloned());
                    list(n)
                }
                L::Dotted(v, t) => {
                    let mut n = vec![arg(0)];
                    n.extend(v.iter().cloned());
                    L::Dotted(Rc::new(n), t)
                }
                t => L::Dotted(Rc::new(vec![arg(0)]), Rc::new(t)),
            },
            "car" | "car-safe" => match arg(0) {
                L::List(v) | L::Dotted(v, _) => v[0].clone(),
                L::Nil => L::Nil,
                o if name == "car-safe" => {
                    let _ = o;
                    L::Nil
                }
                o => return Err(format!("Wrong type argument: listp, {}", prin1(&o))),
            },
            "cdr" | "cdr-safe" => cdr(&arg(0), name == "cdr-safe")?,
            "cadr" => self.call("car", vec![cdr(&arg(0), false)?])?,
            "cddr" => cdr(&cdr(&arg(0), false)?, false)?,
            "caar" => {
                let x = self.call("car", vec![arg(0)])?;
                self.call("car", vec![x])?
            }
            "cdar" => cdr(&self.call("car", vec![arg(0)])?, false)?,
            "nth" | "elt" | "seq-elt" | "aref" => {
                let (i, seq) = if name == "nth" { (arg(0).int()?, arg(1)) } else { (arg(1).int()?, arg(0)) };
                match seq {
                    L::Str(s) => s.chars().nth(i as usize).map_or(L::Nil, |c| L::Int(c as i64)),
                    seq => seq.items()?.get(i.max(0) as usize).cloned().unwrap_or(L::Nil),
                }
            }
            "nthcdr" => {
                let v = arg(1).items()?;
                list(v.get(arg(0).int()?.max(0) as usize..).unwrap_or(&[]).to_vec())
            }
            "last" => {
                let v = arg(0).items()?;
                let n = if arg(1).is_nil() { 1 } else { arg(1).int()? as usize };
                list(v[v.len().saturating_sub(n)..].to_vec())
            }
            "butlast" | "nbutlast" => {
                let v = arg(0).items()?;
                let n = if arg(1).is_nil() { 1 } else { arg(1).int()? as usize };
                list(v[..v.len().saturating_sub(n)].to_vec())
            }
            "take" | "ntake" | "seq-take" => {
                let (n, v) = if name == "seq-take" { (arg(1).int()?, arg(0).items()?) } else { (arg(0).int()?, arg(1).items()?) };
                list(v.into_iter().take(n.max(0) as usize).collect())
            }
            "seq-drop" => list(arg(0).items()?.into_iter().skip(arg(1).int()?.max(0) as usize).collect()),
            "append" => {
                let mut out = vec![];
                for (i, x) in a.iter().enumerate() {
                    if i + 1 == a.len() && !matches!(x, L::List(_) | L::Nil | L::Vector(_)) {
                        return Ok(if out.is_empty() { x.clone() } else { L::Dotted(Rc::new(out), Rc::new(x.clone())) });
                    }
                    match x {
                        L::Str(s) => out.extend(s.chars().map(|c| L::Int(c as i64))),
                        x => out.extend(x.items()?),
                    }
                }
                list(out)
            }
            "reverse" | "nreverse" | "seq-reverse" => match arg(0) {
                L::Str(s) => str_(&s.chars().rev().collect::<String>()),
                x => {
                    let mut v = x.items()?;
                    v.reverse();
                    list(v)
                }
            },
            "length" | "seq-length" | "safe-length" | "proper-list-p" => match arg(0) {
                L::Str(s) => L::Int(s.chars().count() as i64),
                x => L::Int(x.items()?.len() as i64),
            },
            "member" | "memq" | "memql" | "cl-member" => {
                let x = arg(0);
                let v = arg(1).items()?;
                match v.iter().position(|e| if name == "memq" { eq(e, &x) } else if name == "memql" { eql(e, &x) } else { equal(e, &x) }) {
                    Some(i) => list(v[i..].to_vec()),
                    None => L::Nil,
                }
            }
            "assoc" | "assq" | "assoc-string" | "rassoc" | "rassq" => {
                let key = arg(0);
                let mut found = L::Nil;
                for e in arg(1).items()? {
                    let (k, v) = match &e {
                        L::List(p) => (p[0].clone(), cdr(&e, true)?),
                        L::Dotted(p, t) if p.len() == 1 => (p[0].clone(), (**t).clone()),
                        L::Dotted(p, _) => (p[0].clone(), cdr(&e, true)?),
                        _ => continue,
                    };
                    let probe = if name.starts_with("rass") { v } else { k };
                    let hit = match name {
                        "assq" | "rassq" => eq(&probe, &key),
                        "assoc-string" => string_designator(&probe).ok() == string_designator(&key).ok(),
                        _ => equal(&probe, &key),
                    };
                    if hit {
                        found = e;
                        break;
                    }
                }
                found
            }
            "alist-get" => {
                let e = self.call("assq", vec![arg(0), arg(1)])?;
                if e.is_nil() { arg(2) } else { cdr(&e, true)? }
            }
            "plist-get" => {
                let v = arg(0).items()?;
                let mut r = L::Nil;
                for p in v.chunks(2) {
                    if equal(&p[0], &arg(1)) {
                        r = p.get(1).cloned().unwrap_or(L::Nil);
                        break;
                    }
                }
                r
            }
            "plist-member" => {
                let v = arg(0).items()?;
                match v.chunks(2).position(|p| equal(&p[0], &arg(1))) {
                    Some(i) => list(v[i * 2..].to_vec()),
                    None => L::Nil,
                }
            }
            "delete" | "remove" | "delq" | "remq" | "seq-remove" | "cl-remove" => {
                if name == "seq-remove" {
                    let f = arg(0);
                    let mut out = vec![];
                    for e in arg(1).items()? {
                        if self.apply(&f, vec![e.clone()])?.is_nil() {
                            out.push(e);
                        }
                    }
                    list(out)
                } else {
                    let x = arg(0);
                    let q = name.ends_with('q');
                    list(arg(1).items()?.into_iter().filter(|e| if q { !eq(e, &x) } else { !equal(e, &x) }).collect())
                }
            }
            "delete-dups" | "seq-uniq" | "cl-remove-duplicates" => {
                let mut out: Vec<L> = vec![];
                for e in arg(0).items()? {
                    if !out.iter().any(|o| equal(o, &e)) {
                        out.push(e);
                    }
                }
                list(out)
            }
            "number-sequence" => {
                let from = arg(0).num()?;
                if arg(1).is_nil() {
                    return Ok(list(vec![from_num(from)]));
                }
                let to = arg(1).num()?;
                let step = if arg(2).is_nil() { Num::I(1) } else { arg(2).num()? };
                let mut out = vec![];
                let mut x = from;
                while (step.f() > 0.0 && x.f() <= to.f()) || (step.f() < 0.0 && x.f() >= to.f()) {
                    out.push(from_num(x));
                    x = x.add(&step);
                    if out.len() > 1_000_000 {
                        break;
                    }
                }
                list(out)
            }
            "mapcar" | "mapc" | "seq-map" | "mapcan" | "cl-mapcar" => {
                let f = arg(0);
                let seq = match arg(1) {
                    L::Str(s) => s.chars().map(|c| L::Int(c as i64)).collect(),
                    x => x.items()?,
                };
                let mut out = vec![];
                for e in seq {
                    out.push(self.apply(&f, vec![e])?);
                }
                match name {
                    "mapc" => arg(1),
                    "mapcan" => {
                        let mut flat = vec![];
                        for o in out {
                            flat.extend(o.items()?);
                        }
                        list(flat)
                    }
                    _ => list(out),
                }
            }
            "mapconcat" => {
                let f = arg(0);
                let seq = match arg(1) {
                    L::Str(s) => s.chars().map(|c| L::Int(c as i64)).collect(),
                    x => x.items()?,
                };
                let sep = if arg(2).is_nil() { Rc::from("") } else { arg(2).string()? };
                let mut parts = vec![];
                for e in seq {
                    let v = self.apply(&f, vec![e])?;
                    parts.push(match v {
                        L::Str(s) => s.to_string(),
                        L::Nil => String::new(),
                        o => return Err(format!("Wrong type argument: sequencep, {}", prin1(&o))),
                    });
                }
                str_(&parts.join(&sep))
            }
            "seq-filter" | "cl-remove-if-not" | "cl-remove-if" => {
                let (f, s) = if name == "seq-filter" { (arg(0), arg(1)) } else { (arg(0), arg(1)) };
                let keep_true = name != "cl-remove-if";
                let mut out = vec![];
                for e in s.items()? {
                    if self.apply(&f, vec![e.clone()])?.truthy() == keep_true {
                        out.push(e);
                    }
                }
                list(out)
            }
            "cl-reduce" | "seq-reduce" => {
                let f = arg(0);
                let items = arg(1).items()?;
                let mut it = items.into_iter();
                let mut acc = if name == "seq-reduce" {
                    arg(2)
                } else if let Some(pos) = a.iter().position(|x| x.sym_name() == Some(":initial-value")) {
                    a.get(pos + 1).cloned().unwrap_or(L::Nil)
                } else {
                    match it.next() {
                        Some(x) => x,
                        None => return self.apply(&f, vec![]),
                    }
                };
                for e in it {
                    acc = self.apply(&f, vec![acc, e])?;
                }
                acc
            }
            "sort" | "cl-sort" | "seq-sort" => {
                let (seq, pred) = if name == "seq-sort" { (arg(1), arg(0)) } else { (arg(0), arg(1)) };
                let mut v = seq.items()?;
                let mut err = None;
                // Insertion sort keeps the comparisons in the interpreter.
                for i in 1..v.len() {
                    let mut j = i;
                    while j > 0 {
                        match self.apply(&pred, vec![v[j].clone(), v[j - 1].clone()]) {
                            Ok(r) if r.truthy() => {
                                v.swap(j, j - 1);
                                j -= 1;
                            }
                            Ok(_) => break,
                            Err(e) => {
                                err = Some(e);
                                break;
                            }
                        }
                    }
                }
                if let Some(e) = err {
                    return Err(e);
                }
                list(v)
            }
            "apply" => {
                let f = arg(0);
                let mut args: Vec<L> = a.get(1..a.len().saturating_sub(1)).unwrap_or(&[]).to_vec();
                if let Some(last) = a.last().filter(|_| a.len() > 1) {
                    args.extend(last.items()?);
                }
                self.apply(&f, args)?
            }
            "funcall" => {
                let f = arg(0);
                self.apply(&f, a[1..].to_vec())?
            }
            "flatten-tree" | "flatten-list" => {
                fn flat(x: &L, out: &mut Vec<L>) {
                    match x {
                        L::Nil => {}
                        L::List(v) => v.iter().for_each(|e| flat(e, out)),
                        o => out.push(o.clone()),
                    }
                }
                let mut out = vec![];
                flat(&arg(0), &mut out);
                list(out)
            }
            "org-lookup-first" | "org-lookup-last" | "org-lookup-all" => {
                let val = arg(0);
                let sl = arg(1).items()?;
                let rl = if arg(2).is_nil() { sl.clone() } else { arg(2).items()? };
                let p = if arg(3).is_nil() { sym("equal") } else { arg(3) };
                let mut ret = vec![];
                for (s, r) in sl.iter().zip(rl.iter().chain(std::iter::repeat(&L::Nil))) {
                    if self.apply(&p, vec![val.clone(), s.clone()])?.truthy() {
                        ret.push(r.clone());
                        if name == "org-lookup-first" {
                            break;
                        }
                    }
                }
                match name {
                    "org-lookup-all" => list(ret),
                    _ => ret.pop().unwrap_or(L::Nil),
                }
            }
            "orgtbl-ascii-draw" | "orgtbl-uc-draw-grid" | "orgtbl-uc-draw-cont" => {
                let value = match arg(0) {
                    L::Str(s) => string_to_number(&s).f(),
                    x => x.num()?.f(),
                };
                let min = arg(1).num()?.f();
                let max = arg(2).num()?.f();
                let width = if arg(3).is_nil() { 12.0 } else { arg(3).num()?.f() };
                let chars = match name {
                    "orgtbl-uc-draw-grid" => " \u{258F}\u{258E}\u{258D}\u{258C}\u{258B}\u{258A}\u{2589}".to_owned(),
                    "orgtbl-uc-draw-cont" => " \u{258F}\u{258E}\u{258D}\u{258C}\u{258B}\u{258A}\u{2589}\u{2588}".to_owned(),
                    _ if arg(4).truthy() => arg(4).string()?.to_string(),
                    _ => " .:;c!lhVHW".to_owned(),
                };
                str_(&ascii_draw(value, min, max, width, &chars))
            }
            "org-table-time-string-to-seconds" => str_(&super::formula::time_string_to_seconds(&arg(0).string()?)),
            "calc-eval" => {
                let e = match arg(0) {
                    L::Str(s) => s.to_string(),
                    L::List(v) => v[0].string()?.to_string(),
                    o => return Err(format!("Wrong type argument: stringp, {}", prin1(&o))),
                };
                match super::calc::eval_str(&e, &super::calc::Modes::default()) {
                    Ok(s) => str_(&s),
                    Err(m) => list(vec![L::Int(0), str_(&m)]),
                }
            }
            "error" | "user-error" | "signal" => {
                let msg = match a.first() {
                    Some(L::Str(s)) => format(s, &a[1..])?,
                    Some(x) => princ(x),
                    None => String::new(),
                };
                return Err(msg);
            }
            "org-sbe" => return Err("org-sbe needs Org Babel (not available in table formulas here)".into()),
            _ => {
                self.unknown = Some(name.to_owned());
                return Err(format!("Symbol's function definition is void: {name}"));
            }
        })
    }
}

fn cdr(x: &L, safe: bool) -> E {
    Ok(match x {
        L::List(v) => list(v[1..].to_vec()),
        L::Dotted(v, t) if v.len() == 1 => (**t).clone(),
        L::Dotted(v, t) => L::Dotted(Rc::new(v[1..].to_vec()), t.clone()),
        L::Nil => L::Nil,
        _ if safe => L::Nil,
        o => return Err(format!("Wrong type argument: listp, {}", prin1(o))),
    })
}

fn string_designator(x: &L) -> Result<String, String> {
    match x {
        L::Str(s) => Ok(s.to_string()),
        L::Sym(_) | L::Nil | L::T => Ok(x.sym_name().unwrap().to_owned()),
        o => Err(format!("Wrong type argument: stringp, {}", prin1(o))),
    }
}

fn slice_bounds(len: usize, from: &L, to: &L) -> Result<(usize, usize), String> {
    let norm = |x: &L, dflt: usize| -> Result<usize, String> {
        if x.is_nil() {
            return Ok(dflt);
        }
        let i = x.int()?;
        let i = if i < 0 { len as i64 + i } else { i };
        if i < 0 || i as usize > len {
            return Err("Args out of range".into());
        }
        Ok(i as usize)
    };
    let (f, t) = (norm(from, 0)?, norm(to, len)?);
    if f > t {
        return Err("Args out of range".into());
    }
    Ok((f, t))
}

fn round_half_even(f: f64) -> f64 {
    let r = f.round();
    if (f - f.trunc()).abs() == 0.5 { 2.0 * (f / 2.0).round() } else { r }
}

/// An Emacs replacement string (`\&`, `\1`) in regex-crate syntax.
pub fn emacs_replacement(t: &str) -> String {
    let mut out = String::new();
    let mut it = t.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\\' => match it.next() {
                Some('&') => out.push_str("${0}"),
                Some(d) if d.is_ascii_digit() => out.push_str(&format!("${{{d}}}")),
                Some('\\') => out.push('\\'),
                Some(o) => out.push(o),
                None => {}
            },
            '$' => out.push_str("$$"),
            c => out.push(c),
        }
    }
    out
}

/// orgtbl-ascii-draw.
pub fn ascii_draw(value: f64, min: f64, max: f64, width: f64, characters: &str) -> String {
    let chars: Vec<char> = characters.chars().collect();
    let width = width.ceil() as i64;
    let len = chars.len() as i64 - 1;
    let relative = (value - min) / (max - min);
    let steps = round_half_even(relative * width as f64 * len as f64) as i64;
    if steps < 0 {
        "too small".into()
    } else if steps > width * len {
        "too large".into()
    } else {
        let div = steps / len;
        let rem = steps - div * len;
        let mut s: String = std::iter::repeat_n(chars[len as usize], div as usize).collect();
        s.push(chars[rem as usize]);
        s
    }
}

fn is_builtin(s: &str) -> bool {
    let mut i = Interp::new();
    !matches!(i.call(s, vec![]), Err(e) if e.starts_with("Symbol's function definition is void"))
}

/// Convert a parsed option value.
pub fn from_sexp(s: &super::super::sexp::Sexp) -> L {
    use super::super::sexp::Sexp;
    match s {
        Sexp::Nil => L::Nil,
        Sexp::T => L::T,
        Sexp::Int(i) => L::Int(*i),
        Sexp::Float(f) => L::Float(*f),
        Sexp::Str(s) => str_(s),
        Sexp::Sym(s) => sym(s),
        Sexp::List(v) => list(v.iter().map(from_sexp).collect()),
        Sexp::Vector(v) => L::Vector(Rc::new(v.iter().map(from_sexp).collect())),
        Sexp::Dotted(v, t) => L::Dotted(Rc::new(v.iter().map(from_sexp).collect()), Rc::new(from_sexp(t))),
    }
}

// ---- format ----

/// Emacs `format`.
pub fn format(fmt: &str, args: &[L]) -> Result<String, String> {
    let mut out = String::new();
    let mut ai = 0;
    let chars: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c != '%' {
            out.push(c);
            i += 1;
            continue;
        }
        i += 1;
        if chars.get(i) == Some(&'%') {
            out.push('%');
            i += 1;
            continue;
        }
        // %N$ field numbers.
        let save = i;
        let mut num = String::new();
        while i < chars.len() && chars[i].is_ascii_digit() {
            num.push(chars[i]);
            i += 1;
        }
        if chars.get(i) == Some(&'$') && !num.is_empty() {
            ai = num.parse::<usize>().unwrap_or(1) - 1;
            i += 1;
        } else {
            i = save;
        }
        let mut flags = String::new();
        while i < chars.len() && "-+ #0".contains(chars[i]) {
            flags.push(chars[i]);
            i += 1;
        }
        let mut w = String::new();
        while i < chars.len() && chars[i].is_ascii_digit() {
            w.push(chars[i]);
            i += 1;
        }
        let mut prec: Option<usize> = None;
        if chars.get(i) == Some(&'.') {
            i += 1;
            let mut p = String::new();
            while i < chars.len() && chars[i].is_ascii_digit() {
                p.push(chars[i]);
                i += 1;
            }
            prec = Some(p.parse().unwrap_or(0));
        }
        let Some(&conv) = chars.get(i) else {
            return Err("Format string ends in middle of format specifier".into());
        };
        i += 1;
        let arg = args.get(ai).cloned().ok_or("Not enough arguments for format string")?;
        ai += 1;
        let width: usize = w.parse().unwrap_or(0);
        let left = flags.contains('-');
        let zero = flags.contains('0') && !left;
        let plus = flags.contains('+');
        let space = flags.contains(' ');
        let alt = flags.contains('#');
        let numeric = |v: &L| -> Result<Num, String> {
            match v {
                L::Int(i) => Ok(Num::I(*i)),
                L::Float(f) => Ok(Num::F(*f)),
                o => Err(format!("Format specifier doesn’t match argument type: {}", prin1(o))),
            }
        };
        let sign = |neg: bool| if neg { "-" } else if plus { "+" } else if space { " " } else { "" };
        let body = match conv {
            's' | 'S' => {
                let mut s = if conv == 's' { princ(&arg) } else { prin1(&arg) };
                if let Some(p) = prec {
                    s = s.chars().take(p).collect();
                }
                s
            }
            'd' | 'i' => {
                let n = numeric(&arg)?;
                let v = match n {
                    Num::I(i) => i,
                    Num::F(f) => f.trunc() as i64,
                };
                let digits = v.unsigned_abs().to_string();
                format!("{}{}", sign(v < 0), digits)
            }
            'o' | 'x' | 'X' => {
                let n = numeric(&arg)?;
                let v = match n {
                    Num::I(i) => i,
                    Num::F(f) => f.trunc() as i64,
                };
                let a = v.unsigned_abs();
                let s = match conv {
                    'o' => format!("{}{a:o}", if alt { "0" } else { "" }),
                    'x' => format!("{}{a:x}", if alt { "0x" } else { "" }),
                    _ => format!("{}{a:X}", if alt { "0X" } else { "" }),
                };
                format!("{}{}", sign(v < 0), s)
            }
            'c' => char::from_u32(numeric(&arg)?.f() as u32).unwrap_or('?').to_string(),
            'f' | 'e' | 'g' => {
                let x = numeric(&arg)?.f();
                let p = prec.unwrap_or(6);
                let s = match conv {
                    'f' => format!("{:.*}", p, x.abs()),
                    'e' => {
                        let s = format!("{:.*e}", p, x.abs());
                        let (m, e) = s.split_once('e').unwrap();
                        let e: i32 = e.parse().unwrap();
                        format!("{m}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
                    }
                    _ => fmt_g(x.abs(), if p == 0 { 1 } else { p }, alt),
                };
                let s = if x.is_nan() {
                    "nan".to_owned()
                } else if x.is_infinite() {
                    "inf".to_owned()
                } else {
                    s
                };
                format!("{}{}", sign(x < 0.0), s)
            }
            other => return Err(format!("Invalid format operation %{other}")),
        };
        let len = body.chars().count();
        if len >= width {
            out.push_str(&body);
        } else if left {
            out.push_str(&body);
            out.push_str(&" ".repeat(width - len));
        } else if zero && !matches!(conv, 's' | 'S' | 'c') {
            let (s, digits) = match body.chars().next() {
                Some(c @ ('-' | '+' | ' ')) => (c.to_string(), body[1..].to_owned()),
                _ => (String::new(), body.clone()),
            };
            out.push_str(&s);
            out.push_str(&"0".repeat(width - len));
            out.push_str(&digits);
        } else {
            out.push_str(&" ".repeat(width - len));
            out.push_str(&body);
        }
    }
    Ok(out)
}

/// Format a number string as Emacs `(format FMT (string-to-number S))`.
pub fn format_number(fmt: &str, s: &str) -> Result<String, String> {
    format(fmt, &[from_num(string_to_number(s))])
}

// ---- formulas ----

/// Evaluate a table Lisp formula: `(eval (eval (read form)))`, the result
/// as the string inserted in the field. Lisp outside the subset falls
/// back to `emacs --batch`.
pub fn eval_formula(form: &str) -> Result<String, String> {
    let mut i = Interp::new();
    let r = read(form).and_then(|x| {
        let once = i.eval(&x)?;
        i.eval(&once)
    });
    match r {
        Ok(v) => Ok(match v {
            L::Int(_) | L::Float(_) => princ(&v),
            v => princ(&v),
        }),
        Err(e) => {
            if i.unknown.is_some()
                && let Some(out) = emacs_eval(form)
            {
                return Ok(out);
            }
            Err(e)
        }
    }
}

/// Run `form` in `emacs --batch -Q` (Org loaded when the form uses it).
pub fn emacs_eval(form: &str) -> Option<String> {
    let emacs = which("emacs")?;
    let mut expr = String::from("(progn ");
    if form.contains("org") {
        expr.push_str("(ignore-errors (require 'org-table)) ");
    }
    let _ = write!(expr, "(princ (let ((v (eval (eval (car (read-from-string {})))))) (if (numberp v) (number-to-string v) (format \"%s\" v)))))", prin1(&str_(form)));
    let out = std::process::Command::new(emacs).args(["--batch", "-Q", "--eval", &expr]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A program on PATH.
pub fn which(prog: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(prog)).find(|p| p.is_file())
}
