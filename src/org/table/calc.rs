//! A Calc evaluator for table formulas (`calc-eval` with
//! `org-calc-default-modes`).
//!
//! It follows Emacs Calc where table formulas depend on it: the algebraic
//! parser and its operator precedences (`/` binds looser than `*`), exact
//! integers and fractions, decimal floats with `calc-internal-prec`
//! digits and Calc's rounding, the float display formats (normal `float
//! 8`, `fix`, `sci`, `eng`), vectors and the statistics functions,
//! dates `<...>`, HMS forms, `inf`/`uinf`/`nan`, strings as vectors of
//! character codes, and symbolic results with Calc's default
//! simplifications (`z + 1`, `2 z`, `a / b`). Integers are 128-bit (Calc
//! has bignums) and transcendental functions are computed in binary
//! doubles before rounding to the working precision.
//!
//! Functions it does not know stay unevaluated (`foo(3)`), as in Calc;
//! with Emacs installed the whole formula is then given to Emacs Calc.

use std::cmp::Ordering;

/// A Calc value or an unevaluated formula.
#[derive(Clone, Debug, PartialEq)]
pub enum V {
    Int(i128),
    /// A reduced fraction, denominator > 1.
    Frac(i128, i128),
    /// mantissa × 10^exponent.
    Float(i128, i32),
    Vec(Vec<V>),
    Var(String),
    /// An operator or function application: `+ - * / ^ neg % \ eq neq lt
    /// gt leq geq land lor lnot if percent fact |` or a function name.
    Call(String, Vec<V>),
    /// A date: days since Dec 31, 1 BC (fraction = time of day).
    Date(Box<V>),
    /// Hours, minutes, seconds.
    Hms(Box<V>, Box<V>, Box<V>),
    /// Interval with closedness mask (2 = low closed, 1 = high closed).
    Intv(u8, Box<V>, Box<V>),
    Cplx(Box<V>, Box<V>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatFmt {
    Float,
    Fix,
    Sci,
    Eng,
}

/// The Calc modes table formulas set.
#[derive(Clone, Debug, PartialEq)]
pub struct Modes {
    pub prec: i32,
    pub float_fmt: FloatFmt,
    pub figs: i32,
    pub deg: bool,
    pub frac: bool,
    pub symbolic: bool,
    pub units: bool,
}

impl Default for Modes {
    /// org-calc-default-modes.
    fn default() -> Self {
        Modes { prec: 12, float_fmt: FloatFmt::Float, figs: 8, deg: true, frac: false, symbolic: false, units: false }
    }
}

fn call(name: &str, args: Vec<V>) -> V {
    V::Call(name.into(), args)
}

fn var(name: &str) -> V {
    V::Var(name.into())
}

fn pow10(n: u32) -> Option<i128> {
    10i128.checked_pow(n)
}

pub fn numdigs(a: i128) -> i32 {
    let mut a = a.unsigned_abs();
    let mut n = 0;
    while a > 0 {
        a /= 10;
        n += 1;
    }
    n
}

fn scale_left(a: i128, n: i32) -> Option<i128> {
    a.checked_mul(pow10(n as u32)?)
}

fn scale_right(a: i128, n: i32) -> i128 {
    if a < 0 {
        return -scale_right(-a, n);
    }
    match pow10(n as u32) {
        Some(p) => a / p,
        None => 0,
    }
}

/// math-scale-rounding.
fn scale_rounding(a: i128, n: i32) -> i128 {
    if n >= 0 {
        return scale_left(a, n).unwrap_or(i128::MAX);
    }
    if a < 0 {
        return -scale_rounding(-a, n);
    }
    if n == -1 { (a + 5) / 10 } else { (scale_right(a, -1 - n) + 5) / 10 }
}

fn gcd(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// The evaluation context.
pub struct Calc {
    pub m: Modes,
    /// A function Calc has and this evaluator does not.
    pub unknown: Option<String>,
}

impl Calc {
    pub fn new(m: Modes) -> Calc {
        Calc { m, unknown: None }
    }

    fn prec(&self) -> i32 {
        self.m.prec.clamp(3, 18)
    }

    /// math-make-float.
    pub fn make_float(&self, mut m: i128, mut e: i32) -> V {
        if m == 0 {
            return V::Float(0, 0);
        }
        let ldiff = self.prec() - numdigs(m);
        if ldiff < 0 {
            m = scale_rounding(m, ldiff);
            e -= ldiff;
        }
        while m % 10 == 0 {
            m /= 10;
            e += 1;
        }
        V::Float(m, e)
    }

    pub fn from_f64(&self, x: f64) -> V {
        if x == 0.0 {
            return V::Float(0, 0);
        }
        if !x.is_finite() {
            return if x.is_nan() { var("nan") } else if x > 0.0 { var("inf") } else { call("neg", vec![var("inf")]) };
        }
        let p = (self.prec() as usize).min(17);
        let s = format!("{:.*e}", p - 1, x);
        let (mant, exp) = s.split_once('e').unwrap();
        let exp: i32 = exp.parse().unwrap();
        let digits: String = mant.chars().filter(|c| c.is_ascii_digit() || *c == '-').collect();
        let m: i128 = digits.parse().unwrap_or(0);
        self.make_float(m, exp - (p as i32 - 1))
    }

    /// math-float.
    fn float(&self, a: &V) -> Option<(i128, i32)> {
        match a {
            V::Int(i) => match self.make_float(*i, 0) {
                V::Float(m, e) => Some((m, e)),
                _ => None,
            },
            V::Frac(n, d) => match self.div_float((*n, 0), (*d, 0)) {
                V::Float(m, e) => Some((m, e)),
                _ => None,
            },
            V::Float(m, e) => Some((*m, *e)),
            _ => None,
        }
    }

    fn div_float(&self, a: (i128, i32), b: (i128, i32)) -> V {
        let ldiff = (1 + self.prec() - (numdigs(a.0) - numdigs(b.0))).max(0);
        match scale_left(a.0, ldiff) {
            Some(n) => self.make_float(n / b.0, a.1 - b.1 - ldiff),
            None => self.from_f64(to_f64(&V::Float(a.0, a.1)) / to_f64(&V::Float(b.0, b.1))),
        }
    }

    fn add_float(&self, a: (i128, i32), b: (i128, i32)) -> V {
        let (a, b) = if a.1 >= b.1 { (a, b) } else { (b, a) };
        let ediff = a.1 - b.1;
        if ediff >= 2 * self.prec() {
            return V::Float(a.0, a.1);
        }
        match scale_left(a.0, ediff).and_then(|x| x.checked_add(b.0)) {
            Some(s) => self.make_float(s, b.1),
            None => self.from_f64(to_f64(&V::Float(a.0, a.1)) + to_f64(&V::Float(b.0, b.1))),
        }
    }

    fn frac(&self, n: i128, d: i128) -> V {
        let g = gcd(n, d).max(1);
        let (mut n, mut d) = (n / g, d / g);
        if d < 0 {
            n = -n;
            d = -d;
        }
        if d == 1 { V::Int(n) } else { V::Frac(n, d) }
    }

    fn ratio(a: &V) -> Option<(i128, i128)> {
        match a {
            V::Int(i) => Some((*i, 1)),
            V::Frac(n, d) => Some((*n, *d)),
            _ => None,
        }
    }

    // ---- arithmetic ----

    pub fn neg(&self, a: &V) -> V {
        match a {
            V::Int(i) => V::Int(-i),
            V::Frac(n, d) => V::Frac(-n, *d),
            V::Float(m, e) => V::Float(-m, *e),
            V::Vec(v) => V::Vec(v.iter().map(|x| self.neg(x)).collect()),
            V::Date(x) => V::Date(Box::new(self.neg(x))),
            V::Hms(h, m, s) => V::Hms(Box::new(self.neg(h)), Box::new(self.neg(m)), Box::new(self.neg(s))),
            V::Cplx(r, i) => V::Cplx(Box::new(self.neg(r)), Box::new(self.neg(i))),
            V::Call(f, args) if f == "neg" => args[0].clone(),
            V::Call(f, args) if f == "+" => {
                let a = self.neg(&args[0]);
                let b = self.neg(&args[1]);
                self.add(&a, &b)
            }
            V::Call(f, args) if f == "*" && args.first().is_some_and(is_num) => {
                let mut v = args.clone();
                v[0] = self.neg(&v[0]);
                if is_one(&v[0]) {
                    v.remove(0);
                    if v.len() == 1 {
                        return v.pop().unwrap();
                    }
                }
                call("*", v)
            }
            V::Call(f, args) if f == "/" && is_num(&args[0]) => call("/", vec![self.neg(&args[0]), args[1].clone()]),
            V::Var(n) if n == "nan" || n == "uinf" => a.clone(),
            other => call("neg", vec![other.clone()]),
        }
    }

    pub fn add(&self, a: &V, b: &V) -> V {
        use V::*;
        match (a, b) {
            (Int(x), Int(y)) => return x.checked_add(*y).map_or_else(|| self.from_f64(*x as f64 + *y as f64), Int),
            _ => {}
        }
        if is_zero(a) && is_num(b) {
            return if is_float(a) && is_rat(b) { self.as_float(b) } else { b.clone() };
        }
        if is_zero(b) && is_num(a) {
            return if is_float(b) && is_rat(a) { self.as_float(a) } else { a.clone() };
        }
        if let (Some(x), Some(y)) = (Self::ratio(a), Self::ratio(b)) {
            let n = x.0.checked_mul(y.1).and_then(|p| y.0.checked_mul(x.1).and_then(|q| p.checked_add(q)));
            let d = x.1.checked_mul(y.1);
            if let (Some(n), Some(d)) = (n, d) {
                return self.frac(n, d);
            }
            return self.from_f64(to_f64(a) + to_f64(b));
        }
        if let (Some(x), Some(y)) = (self.float(a), self.float(b)) {
            return self.add_float(x, y);
        }
        match (a, b) {
            (Vec(x), Vec(y)) if x.len() == y.len() => return Vec(x.iter().zip(y).map(|(p, q)| self.add(p, q)).collect()),
            (Vec(x), y) if is_scalar(y) => return Vec(x.iter().map(|p| self.add(p, y)).collect()),
            (x, Vec(y)) if is_scalar(x) => return Vec(y.iter().map(|q| self.add(x, q)).collect()),
            (Date(x), Date(y)) => return self.add(x, y),
            (Date(x), y) if is_real(y) => return Date(Box::new(self.add(x, y))),
            (x, Date(y)) if is_real(x) => return Date(Box::new(self.add(x, y))),
            (Hms(..), Hms(..)) => return self.from_hours(&self.add(&self.hours(a), &self.hours(b)), a, b),
            (Hms(..), y) if is_real(y) => return self.from_hours(&self.add(&self.hours(a), y), a, b),
            (x, Hms(..)) if is_real(x) => return self.from_hours(&self.add(x, &self.hours(b)), a, b),
            (Cplx(r1, i1), Cplx(r2, i2)) => return self.cplx(self.add(r1, r2), self.add(i1, i2)),
            (Cplx(r, i), y) if is_real(y) => return self.cplx(self.add(r, y), (**i).clone()),
            (x, Cplx(r, i)) if is_real(x) => return self.cplx(self.add(x, r), (**i).clone()),
            _ => {}
        }
        self.add_symb(a, b)
    }

    pub fn sub(&self, a: &V, b: &V) -> V {
        self.add(a, &self.neg(b))
    }

    fn cplx(&self, r: V, i: V) -> V {
        if is_zero(&i) && !is_float(&i) { r } else { V::Cplx(Box::new(r), Box::new(i)) }
    }

    fn as_float(&self, a: &V) -> V {
        match self.float(a) {
            Some((m, e)) => V::Float(m, e),
            None => a.clone(),
        }
    }

    pub fn mul(&self, a: &V, b: &V) -> V {
        use V::*;
        if let (Int(x), Int(y)) = (a, b) {
            return x.checked_mul(*y).map_or_else(|| self.from_f64(*x as f64 * *y as f64), Int);
        }
        if is_num(a) && is_num(b) {
            if is_zero(a) {
                return if is_float(b) && is_rat(a) { V::Float(0, 0) } else { a.clone() };
            }
            if is_zero(b) {
                return if is_float(a) && is_rat(b) { V::Float(0, 0) } else { b.clone() };
            }
        }
        if let (Some(x), Some(y)) = (Self::ratio(a), Self::ratio(b)) {
            if let (Some(n), Some(d)) = (x.0.checked_mul(y.0), x.1.checked_mul(y.1)) {
                return self.frac(n, d);
            }
            return self.from_f64(to_f64(a) * to_f64(b));
        }
        if let (Some(x), Some(y)) = (self.float(a), self.float(b)) {
            return match x.0.checked_mul(y.0) {
                Some(m) => self.make_float(m, x.1 + y.1),
                None => self.from_f64(to_f64(a) * to_f64(b)),
            };
        }
        match (a, b) {
            (Vec(x), Vec(y)) if x.len() == y.len() && x.iter().all(is_scalar) && y.iter().all(is_scalar) => {
                // Dot product.
                let mut acc = V::Int(0);
                for (p, q) in x.iter().zip(y) {
                    acc = self.add(&acc, &self.mul(p, q));
                }
                return acc;
            }
            (Vec(x), y) if is_scalar(y) => return Vec(x.iter().map(|p| self.mul(p, y)).collect()),
            (x, Vec(y)) if is_scalar(x) => return Vec(y.iter().map(|q| self.mul(x, q)).collect()),
            (Hms(..), y) if is_real(y) => return self.from_hours(&self.mul(&self.hours(a), y), a, b),
            (x, Hms(..)) if is_real(x) => return self.from_hours(&self.mul(x, &self.hours(b)), a, b),
            (Cplx(r1, i1), Cplx(r2, i2)) => {
                let re = self.sub(&self.mul(r1, r2), &self.mul(i1, i2));
                let im = self.add(&self.mul(r1, i2), &self.mul(i1, r2));
                return self.cplx(re, im);
            }
            (Cplx(r, i), y) | (y, Cplx(r, i)) if is_real(y) => return self.cplx(self.mul(r, y), self.mul(i, y)),
            _ => {}
        }
        self.mul_symb(a, b)
    }

    pub fn div(&self, a: &V, b: &V) -> V {
        use V::*;
        if is_zero(b) && is_num(b) {
            if is_zero(a) && is_num(a) {
                return var("nan");
            }
            return call("/", vec![a.clone(), b.clone()]);
        }
        if is_zero(a) && is_num(a) && is_num(b) {
            return if is_float(b) && is_rat(a) { V::Float(0, 0) } else { a.clone() };
        }
        if let (Int(x), Int(y)) = (a, b) {
            if x % y == 0 {
                return Int(x / y);
            }
            if self.m.frac {
                return self.frac(*x, *y);
            }
            return self.div_float((*x, 0), (*y, 0));
        }
        if let (Some(x), Some(y)) = (Self::ratio(a), Self::ratio(b)) {
            if let (Some(n), Some(d)) = (x.0.checked_mul(y.1), x.1.checked_mul(y.0)) {
                return self.frac(n, d);
            }
        }
        if let (Some(x), Some(y)) = (self.float(a), self.float(b)) {
            return self.div_float(x, y);
        }
        match (a, b) {
            (Vec(x), y) if is_scalar(y) => return Vec(x.iter().map(|p| self.div(p, y)).collect()),
            (Hms(..), y) if is_real(y) => return self.from_hours(&self.div(&self.hours(a), y), a, b),
            (Hms(..), Hms(..)) => return self.div(&self.hours(a), &self.hours(b)),
            (Cplx(r, i), y) if is_real(y) => return self.cplx(self.div(r, y), self.div(i, y)),
            _ => {}
        }
        self.div_symb(a, b)
    }

    /// Integer quotient `\` (floor).
    fn idiv(&self, a: &V, b: &V) -> V {
        if let (V::Int(x), V::Int(y)) = (a, b) {
            if *y == 0 {
                return call("\\", vec![a.clone(), b.clone()]);
            }
            return V::Int(x.div_euclid(*y) - if *y < 0 && x.rem_euclid(*y) != 0 { 1 } else { 0 });
        }
        if is_real(a) && is_real(b) {
            let q = self.div(a, b);
            return self.floor(&q);
        }
        call("\\", vec![a.clone(), b.clone()])
    }

    /// `%` modulo (sign of the divisor).
    fn modulo(&self, a: &V, b: &V) -> V {
        if is_zero(b) {
            return call("%", vec![a.clone(), b.clone()]);
        }
        if let (V::Int(x), V::Int(y)) = (a, b) {
            let r = x % y;
            return V::Int(if r != 0 && (r < 0) != (*y < 0) { r + y } else { r });
        }
        if is_real(a) && is_real(b) {
            let q = self.floor(&self.div(a, b));
            return self.sub(a, &self.mul(&q, b));
        }
        call("%", vec![a.clone(), b.clone()])
    }

    fn hours(&self, a: &V) -> V {
        match a {
            V::Hms(h, m, s) => {
                let x = self.add(h, &self.div(m, &V::Int(60)));
                self.add(&x, &self.div(s, &V::Int(3600)))
            }
            other => other.clone(),
        }
    }

    fn from_hours(&self, h: &V, a: &V, b: &V) -> V {
        if !is_real(h) {
            return call("+", vec![a.clone(), b.clone()]);
        }
        let neg = sign(h) < 0;
        let h = if neg { self.neg(h) } else { h.clone() };
        let hh = self.floor(&h);
        let rest = self.mul(&self.sub(&h, &hh), &V::Int(60));
        let mm = self.floor(&rest);
        let ss = self.mul(&self.sub(&rest, &mm), &V::Int(60));
        let ss = match ss {
            V::Float(m, e) => self.make_float(m, e),
            x => x,
        };
        let r = V::Hms(Box::new(hh), Box::new(mm), Box::new(ss));
        if neg { self.neg(&r) } else { r }
    }

    fn floor(&self, a: &V) -> V {
        match a {
            V::Int(_) => a.clone(),
            V::Frac(n, d) => V::Int(n.div_euclid(*d)),
            V::Float(m, e) => {
                if *e >= 0 {
                    V::Int(scale_left(*m, *e).unwrap_or(i128::MAX))
                } else {
                    let p = pow10((-e) as u32).unwrap_or(i128::MAX);
                    V::Int(m.div_euclid(p))
                }
            }
            V::Vec(v) => V::Vec(v.iter().map(|x| self.floor(x)).collect()),
            _ => call("floor", vec![a.clone()]),
        }
    }

    fn ceil(&self, a: &V) -> V {
        if is_real(a) {
            let n = self.neg(a);
            let f = self.floor(&n);
            self.neg(&f)
        } else {
            call("ceil", vec![a.clone()])
        }
    }

    fn trunc(&self, a: &V) -> V {
        if is_real(a) {
            if sign(a) < 0 { self.ceil(a) } else { self.floor(a) }
        } else {
            call("trunc", vec![a.clone()])
        }
    }

    fn round(&self, a: &V) -> V {
        // Round half away from zero.
        if !is_real(a) {
            return call("round", vec![a.clone()]);
        }
        let half = V::Frac(1, 2);
        if sign(a) < 0 {
            let n = self.neg(a);
            let r = self.floor(&self.add_exact(&n, &half));
            self.neg(&r)
        } else {
            self.floor(&self.add_exact(a, &half))
        }
    }

    /// Exact addition of a rational to a float (for rounding).
    fn add_exact(&self, a: &V, r: &V) -> V {
        match (a, r) {
            (V::Float(m, e), V::Frac(n, d)) if *e < 0 => {
                let p = pow10((-e) as u32).unwrap_or(1);
                // m/p + n/d
                self.frac(m * d + n * p, p * d)
            }
            _ => self.add(a, r),
        }
    }

    fn pow(&self, a: &V, b: &V) -> V {
        use V::*;
        match b {
            Int(n) if *n >= 0 => {
                if *n == 0 {
                    return if is_float(a) { V::Float(1, 0) } else if is_num(a) { Int(1) } else { pow_symb(self, a, b) };
                }
                if is_num(a) || matches!(a, V::Vec(_) | V::Hms(..)) {
                    let mut acc = a.clone();
                    for _ in 1..(*n).min(100_000) {
                        acc = self.mul(&acc, a);
                    }
                    return acc;
                }
                return pow_symb(self, a, b);
            }
            Int(n) => {
                if is_num(a) {
                    let p = self.pow(a, &Int(-n));
                    return self.div(&Int(1), &p);
                }
                return pow_symb(self, a, b);
            }
            Frac(n, d) if is_rat(a) && sign(a) >= 0 => {
                // Exact root when there is one.
                if let Some((an, ad)) = Self::ratio(a)
                    && let (Some(rn), Some(rd)) = (iroot(an, *d), iroot(ad, *d))
                {
                    return self.pow(&self.frac(rn, rd), &Int(*n));
                }
                return self.from_f64(to_f64(a).powf(to_f64(b)));
            }
            _ => {}
        }
        if is_real(a) && is_real(b) {
            let x = to_f64(a);
            if x < 0.0 {
                // A complex result.
                let r = x.abs().powf(to_f64(b));
                let t = std::f64::consts::PI * to_f64(b);
                return self.cplx(self.from_f64(r * t.cos()), self.from_f64(r * t.sin()));
            }
            return self.from_f64(x.powf(to_f64(b)));
        }
        pow_symb(self, a, b)
    }

    // ---- symbolic simplification ----

    fn add_symb(&self, a: &V, b: &V) -> V {
        if is_nan(a) || is_nan(b) {
            return var("nan");
        }
        if let (Some(x), Some(y)) = (inf_sign(a), inf_sign(b)) {
            return if x == y && x != 0 { a.clone() } else { var("nan") };
        }
        if inf_sign(a).is_some() && is_real(b) {
            return a.clone();
        }
        if inf_sign(b).is_some() && is_real(a) {
            return b.clone();
        }
        if is_zero(a) && is_rat(a) {
            return b.clone();
        }
        if is_zero(b) && is_rat(b) {
            return a.clone();
        }
        // Combine with the last term (numbers) or any like term.
        let mut terms = sum_terms(a);
        let bterms = sum_terms(b);
        for t in bterms {
            let mut merged = false;
            let n = terms.len();
            for i in (0..n).rev() {
                let last = i == n - 1;
                if is_num(&t) && is_num(&terms[i]) && last {
                    terms[i] = self.add(&terms[i], &t);
                    merged = true;
                    break;
                }
                if !is_num(&t)
                    && let (Some((c1, r1)), Some((c2, r2))) = (split_coef(&terms[i]), split_coef(&t))
                    && r1 == r2
                {
                    let c = self.add(&c1, &c2);
                    terms[i] = self.coef_times(&c, &r1);
                    merged = true;
                    break;
                }
            }
            if !merged {
                terms.push(t);
            }
        }
        terms.retain(|t| !(is_zero(t) && is_rat(t)));
        let mut it = terms.into_iter();
        let Some(first) = it.next() else {
            return V::Int(0);
        };
        it.fold(first, |acc, t| call("+", vec![acc, t]))
    }

    fn coef_times(&self, c: &V, r: &V) -> V {
        if is_zero(c) {
            return if is_float(c) { c.clone() } else { V::Int(0) };
        }
        if is_one(c) && !is_float(c) {
            return r.clone();
        }
        if matches!(c, V::Int(-1)) {
            return call("neg", vec![r.clone()]);
        }
        let mut f = vec![c.clone()];
        f.extend(prod_factors(r));
        call("*", f)
    }

    fn mul_symb(&self, a: &V, b: &V) -> V {
        if is_nan(a) || is_nan(b) {
            return var("nan");
        }
        if let Some(s) = inf_sign(a).or_else(|| inf_sign(b)) {
            let other = if inf_sign(a).is_some() { b } else { a };
            if let Some(o) = inf_sign(other) {
                return if s == 0 || o == 0 { var("uinf") } else if s * o > 0 { var("inf") } else { self.neg(&var("inf")) };
            }
            if is_real(other) {
                let sg = sign(other);
                if sg == 0 {
                    return var("nan");
                }
                if s == 0 {
                    return var("uinf");
                }
                return if s * sg > 0 { var("inf") } else { self.neg(&var("inf")) };
            }
        }
        if is_rat(a) && is_zero(a) {
            return a.clone();
        }
        if is_rat(b) && is_zero(b) {
            return b.clone();
        }
        if is_one(a) && !is_float(a) {
            return b.clone();
        }
        if is_one(b) && !is_float(b) {
            return a.clone();
        }
        // Numbers distribute over sums.
        if is_num(a)
            && let V::Call(f, args) = b
            && f == "+"
        {
            return self.add(&self.mul(a, &args[0]), &self.mul(a, &args[1]));
        }
        if is_num(b)
            && let V::Call(f, args) = a
            && f == "+"
        {
            return self.add(&self.mul(&args[0], b), &self.mul(&args[1], b));
        }
        // Quotients: multiply numerators.
        if let V::Call(f, args) = a
            && f == "/"
        {
            let n = self.mul(&args[0], b);
            return self.div(&n, &args[1]);
        }
        if let V::Call(f, args) = b
            && f == "/"
        {
            let n = self.mul(a, &args[0]);
            return self.div(&n, &args[1]);
        }
        if let V::Call(f, args) = a
            && f == "neg"
        {
            let p = self.mul(&args[0], b);
            return self.neg(&p);
        }
        if let V::Call(f, args) = b
            && f == "neg"
        {
            let p = self.mul(a, &args[0]);
            return self.neg(&p);
        }
        let mut coef = V::Int(1);
        let mut facs: Vec<(V, V)> = vec![];
        for f in prod_factors(a).into_iter().chain(prod_factors(b)) {
            if is_num(&f) {
                coef = self.mul(&coef, &f);
                continue;
            }
            let (base, exp) = base_exp(&f);
            if let Some(slot) = facs.iter_mut().find(|(bb, _)| *bb == base) {
                slot.1 = self.add(&slot.1, &exp);
            } else {
                facs.push((base, exp));
            }
        }
        self.build_product(coef, facs)
    }

    fn build_product(&self, coef: V, facs: Vec<(V, V)>) -> V {
        let mut out = vec![];
        for (b, e) in facs {
            if is_zero(&e) && is_rat(&e) {
                continue;
            }
            out.push(if is_one(&e) && !is_float(&e) { b } else { call("^", vec![b, e]) });
        }
        if out.is_empty() {
            return coef;
        }
        if is_zero(&coef) && is_rat(&coef) {
            return V::Int(0);
        }
        if matches!(coef, V::Int(-1)) {
            let p = if out.len() == 1 { out.pop().unwrap() } else { call("*", out) };
            return call("neg", vec![p]);
        }
        if !(is_one(&coef) && !is_float(&coef)) {
            out.insert(0, coef);
        }
        if out.len() == 1 { out.pop().unwrap() } else { call("*", out) }
    }

    fn div_symb(&self, a: &V, b: &V) -> V {
        if is_nan(a) || is_nan(b) {
            return var("nan");
        }
        if inf_sign(b).is_some() && is_real(a) {
            return V::Int(0);
        }
        if inf_sign(a).is_some() && is_real(b) {
            let s = sign(b);
            return if s > 0 { a.clone() } else { self.neg(a) };
        }
        if is_one(b) && !is_float(b) {
            return a.clone();
        }
        if a == b {
            return V::Int(1);
        }
        if let V::Call(f, args) = a
            && f == "/"
        {
            let d = self.mul(&args[1], b);
            return self.div(&args[0], &d);
        }
        if let V::Call(f, args) = a
            && f == "neg"
        {
            let q = self.div(&args[0], b);
            return self.neg(&q);
        }
        // Cancel common factors and numeric coefficients.
        let (mut ncoef, mut nf) = (V::Int(1), vec![]);
        for f in prod_factors(a) {
            if is_num(&f) {
                ncoef = self.mul(&ncoef, &f);
            } else {
                nf.push(base_exp(&f));
            }
        }
        let (mut dcoef, mut df) = (V::Int(1), vec![]);
        for f in prod_factors(b) {
            if is_num(&f) {
                dcoef = self.mul(&dcoef, &f);
            } else {
                df.push(base_exp(&f));
            }
        }
        let mut rest_d = vec![];
        for (base, e) in df {
            if let Some(slot) = nf.iter_mut().find(|(bb, _)| *bb == base) {
                slot.1 = self.sub(&slot.1, &e);
            } else {
                rest_d.push((base, e));
            }
        }
        // Negative leftovers move to the denominator.
        let mut keep_n = vec![];
        for (base, e) in nf {
            if is_real(&e) && sign(&e) < 0 {
                rest_d.push((base, self.neg(&e)));
            } else {
                keep_n.push((base, e));
            }
        }
        let q = self.div(&ncoef, &dcoef);
        let exact = matches!(q, V::Int(_)) || is_float(&ncoef) || is_float(&dcoef);
        let (ncoef, dcoef) = if exact { (q, V::Int(1)) } else { (ncoef, dcoef) };
        let num = self.build_product(ncoef, keep_n);
        if rest_d.is_empty() && is_one(&dcoef) {
            return num;
        }
        let den = self.build_product(dcoef, rest_d);
        call("/", vec![num, den])
    }

    // ---- evaluation ----

    /// Evaluate a parsed formula.
    pub fn eval(&mut self, x: &V) -> V {
        match x {
            V::Call(f, args) if f == "if" => {
                let c = self.eval(&args[0]);
                match truth(&c) {
                    Some(true) => self.eval(args.get(1).unwrap_or(&V::Int(0))),
                    Some(false) => self.eval(args.get(2).unwrap_or(&V::Int(0))),
                    None => {
                        let rest = args[1..].iter().map(|a| self.eval(a)).collect::<Vec<_>>();
                        let mut v = vec![c];
                        v.extend(rest);
                        call("if", v)
                    }
                }
            }
            V::Call(f, args) => {
                let vals: Vec<V> = args.iter().map(|a| self.eval(a)).collect();
                self.apply(f, vals)
            }
            V::Vec(v) => V::Vec(v.iter().map(|e| self.eval(e)).collect()),
            V::Date(d) => V::Date(Box::new(self.eval(d))),
            V::Float(m, e) => self.make_float(*m, *e),
            other => other.clone(),
        }
    }

    fn unary_f64(&self, name: &str, a: &V, f: &dyn Fn(f64) -> f64) -> V {
        if let V::Vec(v) = a {
            return V::Vec(v.iter().map(|x| self.unary_f64(name, x, f)).collect());
        }
        if !is_real(a) {
            return call(name, vec![a.clone()]);
        }
        self.from_f64(f(to_f64(a)))
    }

    fn to_rad(&self, x: f64) -> f64 {
        if self.m.deg { x.to_radians() } else { x }
    }

    fn from_rad(&self, x: f64) -> f64 {
        if self.m.deg { x.to_degrees() } else { x }
    }

    fn vec_arg(args: &[V]) -> Option<Vec<V>> {
        if args.len() == 1
            && let V::Vec(v) = &args[0]
        {
            return Some(v.clone());
        }
        if args.iter().all(|a| !matches!(a, V::Vec(_))) {
            return Some(args.to_vec());
        }
        // Several vectors: concatenate.
        let mut out = vec![];
        for a in args {
            match a {
                V::Vec(v) => out.extend(v.iter().cloned()),
                x => out.push(x.clone()),
            }
        }
        Some(out)
    }

    fn compare_nums(&self, a: &V, b: &V) -> Option<Ordering> {
        let (a, b) = match (a, b) {
            (V::Date(x), V::Date(y)) => (&**x, &**y),
            _ => (a, b),
        };
        if let (Some(x), Some(y)) = (Self::ratio(a), Self::ratio(b)) {
            return (x.0 * y.1).checked_sub(y.0 * x.1).map(|d| d.cmp(&0));
        }
        if is_real(a) && is_real(b) {
            let d = self.sub(a, b);
            return Some(sign(&d).cmp(&0));
        }
        match (inf_sign(a), inf_sign(b)) {
            (Some(x), None) if x != 0 && is_real(b) => Some(x.cmp(&0)),
            (None, Some(y)) if y != 0 && is_real(a) => Some(0.cmp(&y)),
            _ => None,
        }
    }

    fn equal(&self, a: &V, b: &V) -> Option<bool> {
        match (a, b) {
            (V::Vec(x), V::Vec(y)) => {
                if x.len() != y.len() {
                    return Some(false);
                }
                let mut all = true;
                for (p, q) in x.iter().zip(y) {
                    match self.equal(p, q) {
                        Some(true) => {}
                        Some(false) => return Some(false),
                        None => all = false,
                    }
                }
                if all { Some(true) } else { None }
            }
            (V::Vec(_), y) | (y, V::Vec(_)) if is_real(y) => Some(false),
            _ => {
                if let Some(o) = self.compare_nums(a, b) {
                    return Some(o == Ordering::Equal);
                }
                if is_nan(a) || is_nan(b) {
                    return None;
                }
                if a == b {
                    return Some(true);
                }
                None
            }
        }
    }

    /// Apply function or operator `f`.
    pub fn apply(&mut self, f: &str, a: Vec<V>) -> V {
        let n = a.len();
        let arg = |i: usize| a.get(i).cloned().unwrap_or(V::Int(0));
        let bool_v = |b: bool| V::Int(b as i128);
        let unevaluated = || call(f, a.clone());
        match (f, n) {
            ("+", 2) => self.add(&a[0], &a[1]),
            ("-", 2) => self.sub(&a[0], &a[1]),
            ("*", 2) => self.mul(&a[0], &a[1]),
            ("/", 2) => self.div(&a[0], &a[1]),
            ("^", 2) | ("pow", 2) => self.pow(&a[0], &a[1]),
            ("neg", 1) => self.neg(&a[0]),
            ("ident", 1) => arg(0),
            ("%", 2) | ("mod", 2) => self.modulo(&a[0], &a[1]),
            ("\\", 2) | ("idiv", 2) => self.idiv(&a[0], &a[1]),
            ("percent", 1) => self.div(&a[0], &V::Int(100)),
            ("fact", 1) => match &a[0] {
                V::Int(k) if *k >= 0 => {
                    let mut acc: Option<i128> = Some(1);
                    for i in 2..=*k {
                        acc = acc.and_then(|x| x.checked_mul(i));
                    }
                    acc.map_or_else(|| self.from_f64(gamma(*k as f64 + 1.0)), V::Int)
                }
                x if is_real(x) => self.from_f64(gamma(to_f64(x) + 1.0)),
                _ => unevaluated(),
            },
            ("dfact", 1) => match &a[0] {
                V::Int(k) if *k >= 0 => {
                    let mut acc: i128 = 1;
                    let mut i = *k;
                    while i > 1 {
                        acc = acc.saturating_mul(i);
                        i -= 2;
                    }
                    V::Int(acc)
                }
                _ => unevaluated(),
            },
            ("|", 2) | ("vconcat", 2) => match (&a[0], &a[1]) {
                (V::Vec(x), V::Vec(y)) => V::Vec(x.iter().chain(y).cloned().collect()),
                (V::Vec(x), y) => V::Vec(x.iter().cloned().chain([y.clone()]).collect()),
                (x, V::Vec(y)) => V::Vec([x.clone()].into_iter().chain(y.iter().cloned()).collect()),
                _ => unevaluated(),
            },
            ("eq", 2) | ("neq", 2) => match self.equal(&a[0], &a[1]) {
                Some(e) => bool_v(e == (f == "eq")),
                None => unevaluated(),
            },
            ("lt", 2) | ("gt", 2) | ("leq", 2) | ("geq", 2) => match self.compare_nums(&a[0], &a[1]) {
                Some(o) => bool_v(match f {
                    "lt" => o == Ordering::Less,
                    "gt" => o == Ordering::Greater,
                    "leq" => o != Ordering::Greater,
                    _ => o != Ordering::Less,
                }),
                None => unevaluated(),
            },
            ("land", 2) => match (truth(&a[0]), truth(&a[1])) {
                (Some(false), _) => a[0].clone(),
                (Some(true), _) => a[1].clone(),
                _ => unevaluated(),
            },
            ("lor", 2) => match truth(&a[0]) {
                Some(true) => a[0].clone(),
                Some(false) => a[1].clone(),
                None => unevaluated(),
            },
            ("lnot", 1) => match truth(&a[0]) {
                Some(t) => bool_v(!t),
                None => unevaluated(),
            },
            ("abs", 1) => match &a[0] {
                x if is_real(x) => {
                    if sign(x) < 0 {
                        self.neg(x)
                    } else {
                        x.clone()
                    }
                }
                V::Vec(v) => {
                    // Vector norm.
                    let s = v.iter().fold(V::Int(0), |acc, x| self.add(&acc, &self.mul(x, x)));
                    self.apply("sqrt", vec![s])
                }
                V::Cplx(r, i) => {
                    let s = self.add(&self.mul(r, r), &self.mul(i, i));
                    self.apply("sqrt", vec![s])
                }
                _ if inf_sign(&a[0]).is_some() => var("inf"),
                _ => unevaluated(),
            },
            ("sign", 1) => match &a[0] {
                x if is_real(x) => V::Int(sign(x) as i128),
                _ => unevaluated(),
            },
            ("floor", 1) => self.floor(&a[0]),
            ("ceil", 1) => self.ceil(&a[0]),
            ("trunc", 1) => self.trunc(&a[0]),
            ("round", 1) => self.round(&a[0]),
            ("floor" | "ceil" | "trunc" | "round", 2) => {
                if !is_real(&a[0]) || !matches!(a[1], V::Int(_)) {
                    return unevaluated();
                }
                let V::Int(k) = a[1] else { unreachable!() };
                let scale = self.pow(&V::Int(10), &V::Int(k));
                let x = self.mul(&a[0], &scale);
                let x = match &x {
                    V::Float(m, e) => self.make_float(*m, *e),
                    o => o.clone(),
                };
                let r = self.apply(f, vec![x]);
                let q = self.div(&r, &scale);
                if is_float(&a[0]) { self.as_float(&q) } else { q }
            }
            ("ffloor" | "fceil" | "ftrunc" | "fround", 1) => {
                let r = self.apply(&f[1..], vec![arg(0)]);
                self.as_float(&r)
            }
            ("frac", 1) => match &a[0] {
                V::Float(m, e) if *e < 0 => self.frac(*m, pow10((-e) as u32).unwrap_or(1)),
                x if is_rat(x) => x.clone(),
                _ => unevaluated(),
            },
            ("float", 1) => {
                if is_num(&a[0]) {
                    self.as_float(&a[0])
                } else {
                    unevaluated()
                }
            }
            ("evalv", 1) | ("evalvn", 1) => self.evalv(&a[0]),
            ("inv", 1) => self.div(&V::Int(1), &a[0]),
            ("sqr", 1) => self.mul(&a[0], &a[0]),
            ("sqrt", 1) => self.sqrt(&a[0]),
            ("exp", 1) => {
                if is_zero(&a[0]) && is_rat(&a[0]) {
                    V::Int(1)
                } else if self.m.symbolic && is_rat(&a[0]) {
                    unevaluated()
                } else {
                    self.unary_f64("exp", &a[0], &f64::exp)
                }
            }
            ("exp10", 1) => self.unary_f64("exp10", &a[0], &|x| 10f64.powf(x)),
            ("expm1", 1) => self.unary_f64("expm1", &a[0], &f64::exp_m1),
            ("ln", 1) => {
                if is_one(&a[0]) && is_rat(&a[0]) {
                    return V::Int(0);
                }
                if is_real(&a[0]) && sign(&a[0]) <= 0 {
                    return unevaluated();
                }
                if self.m.symbolic && is_rat(&a[0]) {
                    return unevaluated();
                }
                self.unary_f64("ln", &a[0], &f64::ln)
            }
            ("lnp1", 1) => self.unary_f64("lnp1", &a[0], &f64::ln_1p),
            ("log10", 1) => self.log(&a[0], &V::Int(10), f),
            ("log", 2) => self.log(&a[0], &a[1], f),
            ("log", 1) => self.apply("ln", vec![arg(0)]),
            ("sin" | "cos" | "tan" | "sec" | "csc" | "cot", 1) => self.trig(f, &a[0]),
            ("arcsin" | "arccos" | "arctan", 1) => self.arctrig(f, &a[0]),
            ("arctan2", 2) => {
                if is_real(&a[0]) && is_real(&a[1]) {
                    let r = self.from_rad(to_f64(&a[0]).atan2(to_f64(&a[1])));
                    self.from_f64(r)
                } else {
                    unevaluated()
                }
            }
            ("sinh", 1) => self.unary_f64(f, &a[0], &f64::sinh),
            ("cosh", 1) => self.unary_f64(f, &a[0], &f64::cosh),
            ("tanh", 1) => self.unary_f64(f, &a[0], &f64::tanh),
            ("arcsinh", 1) => self.unary_f64(f, &a[0], &f64::asinh),
            ("arccosh", 1) => self.unary_f64(f, &a[0], &f64::acosh),
            ("arctanh", 1) => self.unary_f64(f, &a[0], &f64::atanh),
            ("deg", 1) => {
                if is_real(&a[0]) {
                    self.from_f64(to_f64(&a[0]).to_degrees())
                } else {
                    unevaluated()
                }
            }
            ("rad", 1) => {
                if is_real(&a[0]) {
                    self.from_f64(to_f64(&a[0]).to_radians())
                } else {
                    unevaluated()
                }
            }
            ("gcd", 2) | ("lcm", 2) => match (&a[0], &a[1]) {
                (V::Int(x), V::Int(y)) => {
                    let g = gcd(*x, *y);
                    if f == "gcd" {
                        V::Int(g)
                    } else if g == 0 {
                        V::Int(0)
                    } else {
                        V::Int((x / g * y).abs())
                    }
                }
                _ => unevaluated(),
            },
            ("choose", 2) | ("perm", 2) => match (&a[0], &a[1]) {
                (V::Int(nn), V::Int(k)) if *k >= 0 && *nn >= *k => {
                    let mut acc: i128 = 1;
                    for i in 0..*k {
                        acc = acc.saturating_mul(nn - i);
                        if f == "choose" {
                            acc /= i + 1;
                        }
                    }
                    V::Int(acc)
                }
                (V::Int(_), V::Int(_)) => V::Int(0),
                _ => unevaluated(),
            },
            ("max" | "min" | "vmax" | "vmin", _) => {
                let Some(items) = (if f.starts_with('v') { Self::vec_arg(&a) } else if a.iter().any(|x| matches!(x, V::Vec(_))) { None } else { Some(a.clone()) })
                else {
                    return unevaluated();
                };
                if items.is_empty() {
                    return if f.ends_with("max") { self.neg(&var("inf")) } else { var("inf") };
                }
                let want = if f.ends_with("max") { Ordering::Greater } else { Ordering::Less };
                let mut best = items[0].clone();
                for x in &items[1..] {
                    match self.compare_nums(x, &best) {
                        Some(o) if o == want => best = x.clone(),
                        Some(_) => {}
                        None => return call(if f.ends_with("max") { "max" } else { "min" }, items),
                    }
                }
                if items.iter().any(is_float) { self.as_float(&best) } else { best }
            }
            ("vsum", _) => {
                let items = Self::vec_arg(&a).unwrap_or_default();
                items.iter().fold(V::Int(0), |acc, x| self.add(&acc, x))
            }
            ("vprod", _) => {
                let items = Self::vec_arg(&a).unwrap_or_default();
                items.iter().fold(V::Int(1), |acc, x| self.mul(&acc, x))
            }
            ("vcount", _) | ("vlen", 1) => match (&a[..], f) {
                ([V::Vec(v)], _) => V::Int(v.len() as i128),
                (_, "vcount") => V::Int(Self::vec_arg(&a).map_or(0, |v| v.len()) as i128),
                _ => V::Int(0),
            },
            ("vmean", _) => {
                let items = Self::vec_arg(&a).unwrap_or_default();
                if items.is_empty() {
                    return unevaluated();
                }
                let s = items.iter().fold(V::Int(0), |acc, x| self.add(&acc, x));
                self.div(&s, &V::Int(items.len() as i128))
            }
            ("vmedian", _) => {
                let mut items = Self::vec_arg(&a).unwrap_or_default();
                if items.is_empty() || !items.iter().all(is_real) {
                    return unevaluated();
                }
                items.sort_by(|x, y| self.compare_nums(x, y).unwrap_or(Ordering::Equal));
                let k = items.len();
                if k % 2 == 1 {
                    items[k / 2].clone()
                } else {
                    let s = self.add(&items[k / 2 - 1], &items[k / 2]);
                    self.div(&s, &V::Int(2))
                }
            }
            ("vsdev" | "vvar" | "vpsdev" | "vpvar", _) => {
                let items = Self::vec_arg(&a).unwrap_or_default();
                let pop = f.starts_with("vp");
                if items.is_empty() || (!pop && items.len() < 2) {
                    return unevaluated();
                }
                let k = V::Int(items.len() as i128);
                let mean = self.div(&items.iter().fold(V::Int(0), |acc, x| self.add(&acc, x)), &k);
                let ss = items.iter().fold(V::Int(0), |acc, x| {
                    let d = self.sub(x, &mean);
                    self.add(&acc, &self.mul(&d, &d))
                });
                let var_ = self.div(&ss, &if pop { k } else { V::Int(items.len() as i128 - 1) });
                if f.ends_with("var") { var_ } else { self.sqrt(&var_) }
            }
            ("vgmean", _) => {
                let items = Self::vec_arg(&a).unwrap_or_default();
                if items.is_empty() {
                    return unevaluated();
                }
                let p = items.iter().fold(V::Int(1), |acc, x| self.mul(&acc, x));
                self.pow(&p, &self.frac(1, items.len() as i128))
            }
            ("vhmean", _) => {
                let items = Self::vec_arg(&a).unwrap_or_default();
                if items.is_empty() {
                    return unevaluated();
                }
                let s = items.iter().fold(V::Int(0), |acc, x| self.add(&acc, &self.div(&V::Int(1), x)));
                self.div(&V::Int(items.len() as i128), &s)
            }
            ("vflat", _) => {
                fn flat(v: &V, out: &mut Vec<V>) {
                    match v {
                        V::Vec(x) => x.iter().for_each(|e| flat(e, out)),
                        o => out.push(o.clone()),
                    }
                }
                let mut out = vec![];
                a.iter().for_each(|x| flat(x, &mut out));
                V::Vec(out)
            }
            ("rev", 1) => match &a[0] {
                V::Vec(v) => V::Vec(v.iter().rev().cloned().collect()),
                _ => unevaluated(),
            },
            ("sort" | "rsort", 1) => match &a[0] {
                V::Vec(v) if v.iter().all(is_real) => {
                    let mut v = v.clone();
                    v.sort_by(|x, y| self.compare_nums(x, y).unwrap_or(Ordering::Equal));
                    if f == "rsort" {
                        v.reverse();
                    }
                    V::Vec(v)
                }
                _ => unevaluated(),
            },
            ("head", 1) => match &a[0] {
                V::Vec(v) if !v.is_empty() => v[0].clone(),
                _ => unevaluated(),
            },
            ("tail", 1) => match &a[0] {
                V::Vec(v) if !v.is_empty() => V::Vec(v[1..].to_vec()),
                _ => unevaluated(),
            },
            ("rhead", 1) => match &a[0] {
                V::Vec(v) if !v.is_empty() => V::Vec(v[..v.len() - 1].to_vec()),
                _ => unevaluated(),
            },
            ("rtail", 1) => match &a[0] {
                V::Vec(v) if !v.is_empty() => v[v.len() - 1].clone(),
                _ => unevaluated(),
            },
            ("cons", 2) => match &a[1] {
                V::Vec(v) => V::Vec([a[0].clone()].into_iter().chain(v.iter().cloned()).collect()),
                _ => unevaluated(),
            },
            ("rcons", 2) => match &a[0] {
                V::Vec(v) => V::Vec(v.iter().cloned().chain([a[1].clone()]).collect()),
                _ => unevaluated(),
            },
            ("subvec", 2 | 3) => match (&a[0], &a[1], a.get(2)) {
                (V::Vec(v), V::Int(s), e) => {
                    let len = v.len() as i128;
                    let norm = |i: i128| if i < 0 { len + 1 + i } else { i };
                    let s = norm(*s).max(1);
                    let e = match e {
                        Some(V::Int(e)) => {
                            if *e == 0 {
                                len + 1
                            } else {
                                norm(*e)
                            }
                        }
                        None => len + 1,
                        _ => return unevaluated(),
                    };
                    let (s, e) = ((s - 1).clamp(0, len) as usize, (e - 1).clamp(0, len) as usize);
                    V::Vec(if s < e { v[s..e].to_vec() } else { vec![] })
                }
                _ => unevaluated(),
            },
            ("index", 1..=3) => match (&a[0], a.get(1), a.get(2)) {
                (V::Int(k), start, step) => {
                    let start = start.cloned().unwrap_or(V::Int(1));
                    let step = step.cloned().unwrap_or(V::Int(1));
                    let mut out = vec![];
                    let mut x = start;
                    for _ in 0..(*k).clamp(0, 100_000) {
                        out.push(x.clone());
                        x = self.add(&x, &step);
                    }
                    V::Vec(out)
                }
                _ => unevaluated(),
            },
            ("cvec", 2) => match &a[1] {
                V::Int(k) => V::Vec(vec![a[0].clone(); (*k).clamp(0, 100_000) as usize]),
                _ => unevaluated(),
            },
            ("find", 2) => match &a[0] {
                V::Vec(v) => V::Int(v.iter().position(|x| self.equal(x, &a[1]) == Some(true)).map_or(0, |i| i as i128 + 1)),
                _ => unevaluated(),
            },
            ("vunique", 1) | ("rdup", 1) => match &a[0] {
                V::Vec(v) => {
                    let mut out: Vec<V> = vec![];
                    for x in v {
                        if !out.contains(x) {
                            out.push(x.clone());
                        }
                    }
                    if f == "rdup" {
                        out.sort_by(|x, y| self.compare_nums(x, y).unwrap_or(Ordering::Equal));
                    }
                    V::Vec(out)
                }
                _ => unevaluated(),
            },
            ("typeof", 1) => self.type_of(&a[0]),
            ("string" | "bstring", 1) => unevaluated(),
            ("date", 1) => match &a[0] {
                V::Date(_) => a[0].clone(),
                x if is_real(x) => V::Date(Box::new(x.clone())),
                _ => unevaluated(),
            },
            ("date", 3..=6) => {
                let ints: Option<Vec<i64>> = a.iter().map(|x| if let V::Int(i) = x { Some(*i as i64) } else { None }).collect();
                match ints {
                    Some(v) => {
                        let days = super::days_from_civil(v[0], v[1] as u32, v[2] as u32) + CALC_EPOCH;
                        let secs = v.get(3).copied().unwrap_or(0) * 3600 + v.get(4).copied().unwrap_or(0) * 60 + v.get(5).copied().unwrap_or(0);
                        let d = if secs == 0 { V::Int(days as i128) } else { self.add(&V::Int(days as i128), &self.frac(secs as i128, 86400)) };
                        V::Date(Box::new(d))
                    }
                    None => unevaluated(),
                }
            }
            ("year" | "month" | "day" | "weekday" | "yearday" | "hour" | "minute" | "second" | "time", 1) => match &a[0] {
                V::Date(d) => {
                    let days = self.floor(d);
                    let V::Int(dn) = days else { return unevaluated() };
                    let (y, mo, dd) = super::civil_from_days(dn as i64 - CALC_EPOCH);
                    let frac = self.sub(d, &V::Int(dn));
                    let secs = self.mul(&frac, &V::Int(86400));
                    let secs_i = match self.round(&secs) {
                        V::Int(s) => s,
                        _ => 0,
                    };
                    match f {
                        "year" => V::Int(y as i128),
                        "month" => V::Int(mo as i128),
                        "day" => V::Int(dd as i128),
                        "weekday" => V::Int((dn as i64).rem_euclid(7) as i128),
                        "yearday" => V::Int((dn as i64 - CALC_EPOCH - super::days_from_civil(y, 1, 1) + 1) as i128),
                        "hour" => V::Int(secs_i / 3600),
                        "minute" => V::Int(secs_i % 3600 / 60),
                        "second" => V::Int(secs_i % 60),
                        _ => V::Hms(Box::new(V::Int(secs_i / 3600)), Box::new(V::Int(secs_i % 3600 / 60)), Box::new(V::Int(secs_i % 60))),
                    }
                }
                _ => unevaluated(),
            },
            ("now", 0) => {
                let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64 + super::local_offset_secs();
                let days = secs.div_euclid(86400) + CALC_EPOCH;
                V::Date(Box::new(self.add(&V::Int(days as i128), &self.frac(secs.rem_euclid(86400) as i128, 86400))))
            }
            ("unixtime", 1) => match &a[0] {
                V::Date(d) => {
                    let s = self.mul(&self.sub(d, &V::Int(CALC_EPOCH as i128)), &V::Int(86400));
                    self.round(&s)
                }
                x if is_real(x) => {
                    let days = self.div(x, &V::Int(86400));
                    V::Date(Box::new(self.add(&days, &V::Int(CALC_EPOCH as i128))))
                }
                _ => unevaluated(),
            },
            ("hms", 3) => V::Hms(Box::new(arg(0)), Box::new(arg(1)), Box::new(arg(2))),
            ("hms", 1) => {
                if is_real(&a[0]) {
                    self.from_hours(&a[0], &a[0], &a[0])
                } else {
                    unevaluated()
                }
            }
            ("re", 1) => match &a[0] {
                V::Cplx(r, _) => (**r).clone(),
                x if is_real(x) => x.clone(),
                _ => unevaluated(),
            },
            ("im", 1) => match &a[0] {
                V::Cplx(_, i) => (**i).clone(),
                x if is_real(x) => V::Int(0),
                _ => unevaluated(),
            },
            ("if", _) => unevaluated(),
            _ => {
                if !KNOWN_SYMBOLIC.contains(&f) {
                    self.unknown = Some(f.to_owned());
                }
                unevaluated()
            }
        }
    }

    fn type_of(&self, x: &V) -> V {
        V::Int(match x {
            V::Int(_) => 1,
            V::Frac(..) => 2,
            V::Float(..) => 3,
            V::Hms(..) => 4,
            V::Cplx(..) => 5,
            V::Intv(..) => 8,
            V::Date(d) => {
                if matches!(**d, V::Int(_)) {
                    10
                } else {
                    11
                }
            }
            V::Var(n) if matches!(n.as_str(), "inf" | "uinf" | "nan") => 12,
            V::Var(_) => 100,
            V::Vec(v) => {
                if !v.is_empty() && v.iter().all(|e| matches!(e, V::Vec(_))) {
                    102
                } else {
                    101
                }
            }
            V::Call(f, _) => {
                return var(match f.as_str() {
                    "+" => "add",
                    "-" => "sub",
                    "*" => "mul",
                    "/" => "div",
                    "^" => "pow",
                    "%" => "mod",
                    "\\" => "idiv",
                    "|" => "vconcat",
                    other => other,
                });
            }
        })
    }

    fn sqrt(&self, a: &V) -> V {
        match a {
            V::Int(i) if *i >= 0 => match iroot(*i, 2) {
                Some(r) => V::Int(r),
                None if self.m.symbolic => call("sqrt", vec![a.clone()]),
                None => self.from_f64((*i as f64).sqrt()),
            },
            V::Frac(n, d) if *n >= 0 => match (iroot(*n, 2), iroot(*d, 2)) {
                (Some(x), Some(y)) => self.frac(x, y),
                _ if self.m.symbolic => call("sqrt", vec![a.clone()]),
                _ => self.from_f64(to_f64(a).sqrt()),
            },
            V::Float(..) if sign(a) >= 0 => {
                // Calc: exact for perfect squares in the working precision.
                self.from_f64(to_f64(a).sqrt())
            }
            x if is_real(x) => {
                let pos = self.neg(x);
                let r = self.sqrt(&pos);
                V::Cplx(Box::new(if is_float(&r) { V::Float(0, 0) } else { V::Int(0) }), Box::new(r))
            }
            V::Vec(v) => V::Vec(v.iter().map(|e| self.sqrt(e)).collect()),
            _ => call("sqrt", vec![a.clone()]),
        }
    }

    fn log(&self, x: &V, b: &V, name: &str) -> V {
        let args = if name == "log10" { vec![x.clone()] } else { vec![x.clone(), b.clone()] };
        if !(is_real(x) && is_real(b)) || sign(x) <= 0 || sign(b) <= 0 {
            return call(name, args);
        }
        if let (V::Int(xi), V::Int(bi)) = (x, b)
            && *bi > 1
        {
            let mut p: i128 = 1;
            let mut k = 0;
            while p < *xi {
                match p.checked_mul(*bi) {
                    Some(q) => p = q,
                    None => break,
                }
                k += 1;
            }
            if p == *xi {
                return V::Int(k);
            }
        }
        if self.m.symbolic && is_rat(x) {
            return call(name, args);
        }
        self.from_f64(to_f64(x).ln() / to_f64(b).ln())
    }

    fn trig(&self, f: &str, a: &V) -> V {
        if !is_real(a) {
            return call(f, vec![a.clone()]);
        }
        // Exact values at multiples of a right angle (degrees mode).
        if self.m.deg
            && let V::Int(d) = a
            && d % 90 == 0
        {
            let q = d.rem_euclid(360) / 90;
            let (s, c): (i128, i128) = [(0, 1), (1, 0), (0, -1), (-1, 0)][q as usize];
            match f {
                "sin" => return V::Int(s),
                "cos" => return V::Int(c),
                "tan" if c != 0 => return V::Int(0),
                _ => {}
            }
        }
        if !self.m.deg && is_zero(a) && is_rat(a) {
            match f {
                "sin" | "tan" => return V::Int(0),
                "cos" | "sec" => return V::Int(1),
                _ => {}
            }
        }
        let x = self.to_rad(to_f64(a));
        let v = match f {
            "sin" => x.sin(),
            "cos" => x.cos(),
            "tan" => x.tan(),
            "sec" => 1.0 / x.cos(),
            "csc" => 1.0 / x.sin(),
            _ => 1.0 / x.tan(),
        };
        self.from_f64(v)
    }

    fn arctrig(&self, f: &str, a: &V) -> V {
        if !is_real(a) {
            return call(f, vec![a.clone()]);
        }
        if is_rat(a) {
            let right = if self.m.deg { Some(90) } else { None };
            let exact = match (f, a) {
                ("arcsin", V::Int(0)) | ("arctan", V::Int(0)) | ("arccos", V::Int(1)) => Some(0),
                ("arcsin", V::Int(1)) | ("arccos", V::Int(0)) => right,
                ("arcsin", V::Int(-1)) => right.map(|r| -r),
                ("arccos", V::Int(-1)) => right.map(|r| 2 * r),
                ("arctan", V::Int(1)) => right.map(|r| r / 2),
                ("arctan", V::Int(-1)) => right.map(|r| -r / 2),
                _ => None,
            };
            if let Some(e) = exact {
                return V::Int(e);
            }
        }
        let x = to_f64(a);
        let v = match f {
            "arcsin" => x.asin(),
            "arccos" => x.acos(),
            _ => x.atan(),
        };
        if v.is_nan() {
            return call(f, vec![a.clone()]);
        }
        self.from_f64(self.from_rad(v))
    }

    /// evalv: substitute the numeric values of pi, e and the like.
    fn evalv(&self, x: &V) -> V {
        match x {
            V::Var(n) => match n.as_str() {
                "pi" => self.from_f64(std::f64::consts::PI),
                "e" => self.from_f64(std::f64::consts::E),
                "phi" => self.from_f64(1.618_033_988_749_895),
                "gamma" => self.from_f64(0.577_215_664_901_532_9),
                _ => x.clone(),
            },
            V::Call(f, args) => {
                let vals: Vec<V> = args.iter().map(|a| self.evalv(a)).collect();
                let mut c = Calc::new(self.m.clone());
                c.apply(f, vals)
            }
            V::Vec(v) => V::Vec(v.iter().map(|e| self.evalv(e)).collect()),
            other => other.clone(),
        }
    }
}

/// Calc's date number of 1970-01-01 (days since Dec 31, 1 BC).
pub const CALC_EPOCH: i64 = 719163;

const KNOWN_SYMBOLIC: &[&str] = &["string", "bstring", "if", "neg", "+", "*", "/", "^"];

fn gamma(x: f64) -> f64 {
    // Lanczos approximation.
    if x < 0.5 {
        return std::f64::consts::PI / ((std::f64::consts::PI * x).sin() * gamma(1.0 - x));
    }
    const G: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    let x = x - 1.0;
    let mut a = G[0];
    let t = x + 7.5;
    for (i, g) in G.iter().enumerate().skip(1) {
        a += g / (x + i as f64);
    }
    (2.0 * std::f64::consts::PI).sqrt() * t.powf(x + 0.5) * (-t).exp() * a
}

fn iroot(x: i128, k: i128) -> Option<i128> {
    if x < 0 {
        return None;
    }
    let r = (x as f64).powf(1.0 / k as f64).round() as i128;
    for c in [r - 1, r, r + 1] {
        if c >= 0 && c.checked_pow(k as u32) == Some(x) {
            return Some(c);
        }
    }
    None
}

fn pow_symb(c: &Calc, a: &V, b: &V) -> V {
    if is_nan(a) || is_nan(b) {
        return var("nan");
    }
    if let V::Call(f, args) = a
        && f == "^"
        && matches!(b, V::Int(_))
    {
        let e = c.mul(&args[1], b);
        return pow_symb(c, &args[0], &e);
    }
    if is_one(b) && !is_float(b) {
        return a.clone();
    }
    call("^", vec![a.clone(), b.clone()])
}

fn sum_terms(x: &V) -> Vec<V> {
    match x {
        V::Call(f, args) if f == "+" => {
            let mut v = sum_terms(&args[0]);
            v.extend(sum_terms(&args[1]));
            v
        }
        o => vec![o.clone()],
    }
}

fn prod_factors(x: &V) -> Vec<V> {
    match x {
        V::Call(f, args) if f == "*" => args.iter().flat_map(prod_factors).collect(),
        o => vec![o.clone()],
    }
}

fn base_exp(x: &V) -> (V, V) {
    match x {
        V::Call(f, args) if f == "^" => (args[0].clone(), args[1].clone()),
        o => (o.clone(), V::Int(1)),
    }
}

/// A term as (numeric coefficient, symbolic rest).
fn split_coef(t: &V) -> Option<(V, V)> {
    match t {
        _ if is_num(t) => None,
        V::Call(f, args) if f == "neg" => split_coef(&args[0]).map(|(c, r)| {
            let n = match c {
                V::Int(i) => V::Int(-i),
                V::Frac(n, d) => V::Frac(-n, d),
                V::Float(m, e) => V::Float(-m, e),
                o => o,
            };
            (n, r)
        }),
        V::Call(f, args) if f == "*" && is_num(&args[0]) => {
            let rest = if args.len() == 2 { args[1].clone() } else { call("*", args[1..].to_vec()) };
            Some((args[0].clone(), rest))
        }
        V::Var(n) if matches!(n.as_str(), "inf" | "uinf" | "nan") => None,
        o => Some((V::Int(1), o.clone())),
    }
}

pub fn is_num(x: &V) -> bool {
    matches!(x, V::Int(_) | V::Frac(..) | V::Float(..))
}

fn is_real(x: &V) -> bool {
    is_num(x)
}

fn is_rat(x: &V) -> bool {
    matches!(x, V::Int(_) | V::Frac(..))
}

fn is_float(x: &V) -> bool {
    matches!(x, V::Float(..))
}

fn is_scalar(x: &V) -> bool {
    !matches!(x, V::Vec(_))
}

fn is_zero(x: &V) -> bool {
    matches!(x, V::Int(0) | V::Float(0, _))
}

fn is_one(x: &V) -> bool {
    matches!(x, V::Int(1) | V::Float(1, 0))
}

fn is_nan(x: &V) -> bool {
    matches!(x, V::Var(n) if n == "nan")
}

/// The direction of an infinity: 1, -1, or 0 (uinf).
fn inf_sign(x: &V) -> Option<i32> {
    match x {
        V::Var(n) if n == "inf" => Some(1),
        V::Var(n) if n == "uinf" => Some(0),
        V::Call(f, a) if f == "neg" => inf_sign(&a[0]).map(|s| -s),
        _ => None,
    }
}

fn sign(x: &V) -> i32 {
    match x {
        V::Int(i) => i.signum() as i32,
        V::Frac(n, _) => n.signum() as i32,
        V::Float(m, _) => m.signum() as i32,
        _ => 0,
    }
}

fn truth(x: &V) -> Option<bool> {
    if is_num(x) { Some(!is_zero(x) && !matches!(x, V::Frac(0, _))) } else { None }
}

pub fn to_f64(x: &V) -> f64 {
    match x {
        V::Int(i) => *i as f64,
        V::Frac(n, d) => *n as f64 / *d as f64,
        V::Float(m, e) => format!("{m}e{e}").parse().unwrap_or(f64::NAN),
        _ => f64::NAN,
    }
}

// ---- parser ----

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(String),
    Sym(String),
    Str(String),
    Punc(String),
    End,
}

struct Parser {
    toks: Vec<(Tok, bool)>,
    i: usize,
    /// Inside brackets: whitespace separates elements.
    keep_spaces: u32,
}

fn tokenize(s: &str) -> Result<Vec<(Tok, bool)>, String> {
    // `..` becomes `\dots` (math-read-exprs).
    let chars: Vec<char> = s.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    let mut space = false;
    const PUNCS: &[&str] = &[
        "\\dots", "\\ldots", "&&&", "|||", "!!!", "~=", "<=", ">=", "<>", "/=", "+/-", "**", "<<", ">>", "==", "!=", "&&", "||", "!!", ":=", "::", "=>",
    ];
    while i < chars.len() {
        let c = chars[i];
        if c == ' ' || c == '\t' || c == '\n' {
            space = true;
            i += 1;
            continue;
        }
        let rest: String = chars[i..].iter().collect();
        if c == '.' && chars.get(i + 1) == Some(&'.') && chars.get(i + 2) != Some(&'.') {
            out.push((Tok::Punc("\\dots".into()), space));
            space = false;
            i += 2;
            continue;
        }
        if c.is_alphabetic() {
            let mut j = i;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '\'' || chars[j] == '#') {
                j += 1;
            }
            let name: String = chars[i..j].iter().collect::<String>().replace('#', "-");
            out.push((Tok::Sym(name), space));
            space = false;
            i = j;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit)) {
            let m = NUMBER_TOKEN.find(&rest).map_or(1, |m| m.as_str().chars().count()).max(1);
            out.push((Tok::Num(chars[i..i + m].iter().collect()), space));
            space = false;
            i += m;
            continue;
        }
        if c == '"' {
            let mut j = i + 1;
            let mut text = String::new();
            while j < chars.len() && chars[j] != '"' {
                if chars[j] == '\\' && j + 1 < chars.len() {
                    j += 1;
                    text.push(match chars[j] {
                        'n' => '\n',
                        't' => '\t',
                        'e' => '\x1b',
                        o => o,
                    });
                } else {
                    text.push(chars[j]);
                }
                j += 1;
            }
            out.push((Tok::Str(text), space));
            space = false;
            i = (j + 1).min(chars.len());
            continue;
        }
        if c == '$' || c == '#' {
            return Err(if c == '$' { "$'s not allowed in this context".into() } else { "#'s not allowed in this context".into() });
        }
        if let Some(p) = PUNCS.iter().find(|p| rest.starts_with(**p)) {
            out.push((Tok::Punc((*p).into()), space));
            space = false;
            i += p.chars().count();
            continue;
        }
        out.push((Tok::Punc(c.to_string()), space));
        space = false;
        i += 1;
    }
    out.push((Tok::End, space));
    Ok(out)
}

static NUMBER_TOKEN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r#"^_?([0-9]+\.?0*@ *)?([0-9]+\.?0*' *)?(0*([2-9]|1[0-4])(##?|\^\^)[0-9a-dA-D.]+[eE][-+_]?[0-9]+|0*([2-9]|[0-2][0-9]|3[0-6])(##?|\^\^)[0-9a-zA-Z:.]+|[0-9]+:[0-9:]+|[0-9.]+([eE][-+_]?[0-9]+)?"?)?"#).unwrap()
});

/// Binary or postfix operators: (token, name, left, right; -1 = postfix).
const OPS: &[(&str, &str, i32, i32)] = &[
    ("%", "percent", 1100, -1),
    ("!!", "dfact", 210, -1),
    ("!", "fact", 210, -1),
    ("^", "^", 201, 200),
    ("**", "^", 201, 200),
    ("*", "*", 196, 195),
    ("/", "/", 190, 191),
    ("%", "%", 190, 191),
    ("\\", "\\", 190, 191),
    ("+", "+", 180, 181),
    ("-", "-", 180, 181),
    ("|", "|", 170, 171),
    ("<", "lt", 160, 161),
    (">", "gt", 160, 161),
    ("<=", "leq", 160, 161),
    (">=", "geq", 160, 161),
    ("=", "eq", 160, 161),
    ("==", "eq", 160, 161),
    ("!=", "neq", 160, 161),
    ("&&", "land", 110, 111),
    ("||", "lor", 100, 101),
    ("?", "if", 91, 90),
];

impl Parser {
    fn tok(&self) -> &Tok {
        &self.toks[self.i].0
    }

    fn space_before(&self) -> bool {
        self.toks[self.i].1
    }

    fn data(&self) -> &str {
        match self.tok() {
            Tok::Punc(p) => p,
            _ => "",
        }
    }

    fn next(&mut self) {
        if self.i + 1 < self.toks.len() {
            self.i += 1;
        }
    }

    fn factor_after(&self) -> bool {
        let n = &self.toks[(self.i + 1).min(self.toks.len() - 1)].0;
        match n {
            Tok::Num(_) | Tok::Sym(_) | Tok::Str(_) => true,
            Tok::Punc(p) => matches!(p.as_str(), "-" | "+" | "!" | "(" | "[" | "{"),
            Tok::End => false,
        }
    }

    fn expr(&mut self, prec: i32) -> Result<V, String> {
        let mut x = self.factor()?;
        loop {
            let op: Option<(&str, i32, i32)> = match self.tok().clone() {
                Tok::Punc(p) => {
                    let cands: Vec<_> = OPS.iter().filter(|o| o.0 == p).collect();
                    match cands.len() {
                        0 => None,
                        1 => Some((cands[0].1, cands[0].2, cands[0].3)),
                        _ => {
                            // `%`: postfix percent unless a factor follows.
                            let c = if self.factor_after() { cands[1] } else { cands[0] };
                            Some((c.1, c.2, c.3))
                        }
                    }
                }
                _ => None,
            };
            let juxt = op.is_none()
                && match self.tok() {
                    Tok::Num(_) | Tok::Sym(_) => true,
                    Tok::Punc(p) => p == "(" || p == "[",
                    _ => false,
                }
                && !(self.keep_spaces > 0 && self.space_before());
            let (name, left, right) = match op {
                Some(o) => o,
                None if juxt => ("*", 196, 195),
                None => break,
            };
            if left < prec {
                break;
            }
            if !juxt {
                self.next();
            }
            if right == -1 {
                x = call(name, vec![x]);
                continue;
            }
            if name == "if" {
                let then = self.expr(0)?;
                if self.data() != ":" {
                    return Err("Expected `:'".into());
                }
                self.next();
                let els = self.expr(right)?;
                x = call("if", vec![x, then, els]);
                continue;
            }
            let rhs = self.expr(right)?;
            x = call(name, vec![x, rhs]);
        }
        Ok(x)
    }

    fn list(&mut self, close: &str) -> Result<Vec<V>, String> {
        let mut v = vec![];
        if self.data() == close {
            return Ok(v);
        }
        loop {
            v.push(self.expr(0)?);
            if self.data() == "," {
                self.next();
                continue;
            }
            break;
        }
        Ok(v)
    }

    fn factor(&mut self) -> Result<V, String> {
        match self.tok().clone() {
            Tok::Num(s) => {
                self.next();
                read_number(&s).ok_or_else(|| "Bad format".to_string())
            }
            Tok::Str(s) => {
                self.next();
                Ok(V::Vec(s.chars().map(|c| V::Int(c as i128)).collect()))
            }
            Tok::Sym(name) => {
                self.next();
                if self.data() == "(" {
                    self.next();
                    let saved = std::mem::take(&mut self.keep_spaces);
                    let args = self.list(")")?;
                    self.keep_spaces = saved;
                    if self.data() != ")" && *self.tok() != Tok::End {
                        return Err("Expected `)'".into());
                    }
                    self.next();
                    return Ok(call(&name, args));
                }
                Ok(V::Var(name))
            }
            Tok::Punc(p) => match p.as_str() {
                "-" | "+" | "!" => {
                    self.next();
                    let v = self.expr(if p == "!" { 1000 } else { 197 })?;
                    Ok(match p.as_str() {
                        "-" => match v {
                            V::Int(i) => V::Int(-i),
                            V::Frac(n, d) => V::Frac(-n, d),
                            V::Float(m, e) => V::Float(-m, e),
                            o => call("neg", vec![o]),
                        },
                        "+" => v,
                        _ => call("lnot", vec![v]),
                    })
                }
                "(" => {
                    self.next();
                    let saved = std::mem::take(&mut self.keep_spaces);
                    let r = (|| -> Result<V, String> {
                        if self.data() == "\\dots" {
                            self.next();
                            let hi = self.expr(0)?;
                            return Ok(V::Intv(0, Box::new(call("neg", vec![var("inf")])), Box::new(hi)));
                        }
                        let e = self.expr(0)?;
                        match self.data() {
                            "," => {
                                self.next();
                                let im = self.expr(0)?;
                                Ok(V::Cplx(Box::new(e), Box::new(im)))
                            }
                            "\\dots" => {
                                self.next();
                                let hi = if matches!(self.data(), ")" | "]") { var("inf") } else { self.expr(0)? };
                                let mask = if self.data() == ")" { 0 } else { 1 };
                                Ok(V::Intv(mask, Box::new(e), Box::new(hi)))
                            }
                            _ => Ok(e),
                        }
                    })();
                    self.keep_spaces = saved;
                    let e = r?;
                    if !matches!(self.data(), ")" | "]") && *self.tok() != Tok::End {
                        return Err("Expected `)'".into());
                    }
                    self.next();
                    Ok(e)
                }
                "[" => {
                    self.next();
                    self.keep_spaces += 1;
                    let r = self.brackets();
                    self.keep_spaces -= 1;
                    r
                }
                "<" => {
                    // A date form <...>.
                    let mut text = String::new();
                    self.next();
                    while self.data() != ">" && *self.tok() != Tok::End {
                        if self.space_before() && !text.is_empty() {
                            text.push(' ');
                        }
                        text.push_str(&match self.tok() {
                            Tok::Num(s) | Tok::Sym(s) | Tok::Punc(s) => s.clone(),
                            Tok::Str(s) => s.clone(),
                            Tok::End => String::new(),
                        });
                        self.next();
                    }
                    self.next();
                    parse_date(&text).ok_or_else(|| "Bad format".to_string())
                }
                _ => Err("Expected a number".into()),
            },
            Tok::End => Err("Expected a number".into()),
        }
    }

    fn brackets(&mut self) -> Result<V, String> {
        let mut rows: Vec<Vec<V>> = vec![vec![]];
        if self.data() == "]" {
            self.next();
            return Ok(V::Vec(vec![]));
        }
        loop {
            let e = self.expr(0)?;
            if self.data() == "\\dots" && rows.len() == 1 && rows[0].is_empty() {
                self.next();
                let hi = self.expr(0)?;
                let mask = 2 | if self.data() == "]" { 1 } else { 0 };
                self.next();
                return Ok(V::Intv(mask, Box::new(e), Box::new(hi)));
            }
            rows.last_mut().unwrap().push(e);
            match self.data() {
                "," => self.next(),
                ";" => {
                    self.next();
                    rows.push(vec![]);
                }
                "]" => {
                    self.next();
                    break;
                }
                _ if *self.tok() == Tok::End => break,
                _ if self.space_before() => {}
                _ => return Err("Expected `]'".into()),
            }
        }
        if rows.len() == 1 {
            Ok(V::Vec(rows.pop().unwrap()))
        } else {
            Ok(V::Vec(rows.into_iter().map(V::Vec).collect()))
        }
    }
}

/// math-read-number.
fn read_number(s: &str) -> Option<V> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix('_') {
        return read_number(rest).map(|v| Calc::new(Modes::default()).neg(&v));
    }
    if s.chars().all(|c| c.is_ascii_digit()) && !s.is_empty() {
        return s.parse::<i128>().ok().map(V::Int).or_else(|| Some(Calc::new(Modes::default()).from_f64(s.parse().ok()?)));
    }
    // HMS: 1@ 2' 3"
    if s.contains('@') || s.contains('\'') {
        let re = regex::Regex::new(r#"^(?:([0-9.]+)@ *)?(?:([0-9.]+)' *)?(?:([0-9.]+)"?)?$"#).ok()?;
        let m = re.captures(s)?;
        let part = |i: usize| m.get(i).map_or(Some(V::Int(0)), |x| read_number(x.as_str()));
        return Some(V::Hms(Box::new(part(1)?), Box::new(part(2)?), Box::new(part(3)?)));
    }
    // Radix: 16#FF.
    if let Some((r, digits)) = s.split_once('#') {
        let radix: u32 = r.parse().ok()?;
        let digits = digits.trim_start_matches('#');
        return i128::from_str_radix(digits, radix).ok().map(V::Int);
    }
    // Fractions a:b and a:b:c.
    if s.contains(':') {
        let parts: Vec<i128> = s.split(':').map(|p| p.parse().ok()).collect::<Option<_>>()?;
        let c = Calc::new(Modes::default());
        return match parts.as_slice() {
            [n, d] if *d != 0 => Some(c.frac(*n, *d)),
            [w, n, d] if *d != 0 => Some(c.add(&V::Int(*w), &c.frac(*n, *d))),
            _ => None,
        };
    }
    let s = s.trim_end_matches('"');
    // Decimal and exponent.
    let (mant, exp) = match s.find(['e', 'E']) {
        Some(i) => (&s[..i], s[i + 1..].replace('_', "-").parse::<i32>().ok()?),
        None => (s, 0),
    };
    let (ip, fp) = match mant.split_once('.') {
        Some((a, b)) => (a, b),
        None if s.contains(['e', 'E']) => (mant, ""),
        None => return None,
    };
    if ip.is_empty() && fp.is_empty() {
        return None;
    }
    let digits = format!("{ip}{fp}");
    let digits = digits.trim_start_matches('0');
    let m: i128 = if digits.is_empty() { 0 } else { digits.parse().ok()? };
    Some(V::Float(m, exp - fp.len() as i32))
}

static DATE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^\s*([+-]?[0-9]{1,4})\s*-\s*([0-9]{1,2})\s*-\s*([0-9]{1,2})(?:\s+[A-Za-z]+\.?)?(?:\s+([0-9]{1,2})\s*:\s*([0-9]{2})(?:\s*:\s*([0-9]{2}))?)?\s*$").unwrap()
});

fn parse_date(text: &str) -> Option<V> {
    let m = DATE_RE.captures(text)?;
    let y: i64 = m[1].parse().ok()?;
    let mo: u32 = m[2].parse().ok()?;
    let d: u32 = m[3].parse().ok()?;
    let days = super::days_from_civil(y, mo, d) + CALC_EPOCH;
    let secs: i64 = m.get(4).map_or(0, |h| h.as_str().parse::<i64>().unwrap_or(0) * 3600)
        + m.get(5).map_or(0, |x| x.as_str().parse::<i64>().unwrap_or(0) * 60)
        + m.get(6).map_or(0, |x| x.as_str().parse::<i64>().unwrap_or(0));
    let c = Calc::new(Modes::default());
    Some(V::Date(Box::new(if secs == 0 { V::Int(days as i128) } else { c.add(&V::Int(days as i128), &c.frac(secs as i128, 86400)) })))
}

/// Parse a Calc algebraic formula.
pub fn parse(s: &str) -> Result<V, String> {
    let toks = tokenize(s)?;
    let mut p = Parser { toks, i: 0, keep_spaces: 0 };
    let v = p.expr(0)?;
    if *p.tok() != Tok::End {
        return Err("Syntax error".into());
    }
    Ok(v)
}

// ---- display ----

impl Calc {
    /// math-format-number for floats.
    fn format_float(&self, m: i128, e: i32) -> String {
        if m < 0 {
            return format!("-{}", self.format_float(-m, e));
        }
        let (mut mant, mut exp) = (m, e);
        let fmt = self.m.float_fmt;
        let mut figs = self.m.figs;
        let prec = self.prec();
        if fmt == FloatFmt::Fix && ({
            if figs < 0 {
                figs = -figs;
                true
            } else {
                exp + numdigs(mant) > -figs
            }
        }) {
            let m2 = scale_rounding(mant, exp + figs);
            let mut s = m2.to_string();
            if s.len() as i32 <= figs {
                s = format!("{}{s}", "0".repeat((1 + figs - s.len() as i32) as usize));
            }
            return if figs > 0 {
                let k = s.len() - figs as usize;
                format!("{}.{}", &s[..k], &s[k..])
            } else {
                format!("{s}.")
            };
        }
        if figs < 0 {
            figs += prec;
        }
        if figs > 0 {
            let adj = figs - numdigs(mant);
            if adj < 0 {
                mant = scale_rounding(mant, adj);
                exp -= adj;
            }
        }
        let mut s = mant.to_string();
        let len = s.len() as i32;
        let dpos = exp + len;
        if fmt == FloatFmt::Float && dpos <= prec && dpos >= -1 {
            if dpos == 0 {
                format!("0.{s}")
            } else if exp <= 0 && dpos > 0 {
                format!("{}.{}", &s[..dpos as usize], &s[dpos as usize..])
            } else if exp > 0 {
                format!("{s}{}.", "0".repeat(exp as usize))
            } else {
                format!("0.{}{s}", "0".repeat((-dpos) as usize))
            }
        } else {
            let eadj = exp + len;
            let scale = if fmt == FloatFmt::Eng { 1 + (eadj + 300002).rem_euclid(3) } else { 1 };
            if scale > s.len() as i32 {
                s.push_str(&"0".repeat((scale - s.len() as i32) as usize));
            }
            if scale < s.len() as i32 {
                s = format!("{}.{}", &s[..scale as usize], &s[scale as usize..]);
            }
            format!("{s}e{}", eadj - scale)
        }
    }

    fn format_date(&self, d: &V) -> String {
        let days = match self.floor(d) {
            V::Int(i) => i as i64,
            _ => return format!("date({})", self.format(d)),
        };
        let (y, mo, da) = super::civil_from_days(days - CALC_EPOCH);
        let wd = super::weekday_name(days - CALC_EPOCH);
        let year = if y < 1000 { format!("{}{y}", if y >= 0 { "+" } else { "" }) } else { y.to_string() };
        let mut s = format!("<{year}-{mo:02}-{da:02} {wd}");
        let frac = self.sub(d, &V::Int(days as i128));
        if !is_zero(&frac) {
            let secs = match self.round(&self.mul(&frac, &V::Int(86400))) {
                V::Int(x) => x,
                _ => 0,
            };
            s.push_str(&format!(" {:02}:{:02}", secs / 3600, secs % 3600 / 60));
        }
        s.push('>');
        s
    }

    /// The displayed form of a value (calc-eval's result string).
    pub fn format(&self, v: &V) -> String {
        self.fmt(v, 0)
    }

    fn fmt(&self, v: &V, ctx: i32) -> String {
        let paren = |s: String, p: i32| if ctx > p { format!("({s})") } else { s };
        match v {
            V::Int(i) => {
                if *i < 0 && ctx > 197 {
                    format!("({i})")
                } else {
                    i.to_string()
                }
            }
            V::Frac(n, d) => format!("{n}:{d}"),
            V::Float(m, e) => {
                let s = self.format_float(*m, *e);
                if *m < 0 && ctx > 197 { format!("({s})") } else { s }
            }
            V::Var(n) => n.clone(),
            V::Vec(items) => format!("[{}]", items.iter().map(|x| self.fmt(x, 0)).collect::<Vec<_>>().join(", ")),
            V::Date(d) => self.format_date(d),
            V::Hms(h, m, s) => format!("{}@ {}' {}\"", self.fmt(h, 0), self.fmt(m, 0), self.fmt(s, 0)),
            V::Cplx(r, i) => format!("({}, {})", self.fmt(r, 0), self.fmt(i, 0)),
            V::Intv(mask, lo, hi) => format!(
                "{}{} .. {}{}",
                if mask & 2 != 0 { '[' } else { '(' },
                self.fmt(lo, 0),
                self.fmt(hi, 0),
                if mask & 1 != 0 { ']' } else { ')' }
            ),
            V::Call(f, a) => match (f.as_str(), a.len()) {
                ("+", 2) => {
                    let l = self.fmt(&a[0], 180);
                    let (neg, r) = match negated(&a[1]) {
                        Some(n) => (true, n),
                        None => (false, a[1].clone()),
                    };
                    paren(format!("{l} {} {}", if neg { '-' } else { '+' }, self.fmt(&r, 181)), 180)
                }
                ("-", 2) => paren(format!("{} - {}", self.fmt(&a[0], 180), self.fmt(&a[1], 181)), 180),
                ("neg", 1) => paren(format!("-{}", self.fmt(&a[0], 197)), 197),
                ("*", _) => {
                    let parts: Vec<String> = a.iter().map(|x| self.fmt(x, 196)).collect();
                    paren(parts.join(" "), 196)
                }
                ("/", 2) => {
                    let den = match &a[1] {
                        V::Call(g, _) if g == "*" || g == "/" => format!("({})", self.fmt(&a[1], 0)),
                        d => self.fmt(d, 191),
                    };
                    paren(format!("{} / {den}", self.fmt(&a[0], 190)), 190)
                }
                ("%", 2) => paren(format!("{} % {}", self.fmt(&a[0], 190), self.fmt(&a[1], 191)), 190),
                ("\\", 2) => paren(format!("{} \\ {}", self.fmt(&a[0], 190), self.fmt(&a[1], 191)), 190),
                ("^", 2) => {
                    let e = match &a[1] {
                        x @ (V::Int(_) | V::Float(..)) => self.fmt(x, 0),
                        x => self.fmt(x, 200),
                    };
                    paren(format!("{}^{e}", self.fmt(&a[0], 201)), 200)
                }
                ("|", 2) => paren(format!("{} | {}", self.fmt(&a[0], 170), self.fmt(&a[1], 171)), 170),
                ("eq" | "neq" | "lt" | "gt" | "leq" | "geq", 2) => {
                    let op = match f.as_str() {
                        "eq" => "=",
                        "neq" => "!=",
                        "lt" => "<",
                        "gt" => ">",
                        "leq" => "<=",
                        _ => ">=",
                    };
                    paren(format!("{} {op} {}", self.fmt(&a[0], 160), self.fmt(&a[1], 161)), 160)
                }
                ("land", 2) => paren(format!("{} && {}", self.fmt(&a[0], 110), self.fmt(&a[1], 111)), 110),
                ("lor", 2) => paren(format!("{} || {}", self.fmt(&a[0], 100), self.fmt(&a[1], 101)), 100),
                ("lnot", 1) => paren(format!("!{}", self.fmt(&a[0], 1000)), 1000),
                ("if", 3) => paren(format!("{} ? {} : {}", self.fmt(&a[0], 91), self.fmt(&a[1], 0), self.fmt(&a[2], 90)), 91),
                ("percent", 1) => format!("{}%", self.fmt(&a[0], 1100)),
                ("fact", 1) => format!("{}!", self.fmt(&a[0], 210)),
                ("string" | "bstring", 1) => match &a[0] {
                    V::Vec(items) if items.iter().all(|x| matches!(x, V::Int(_))) => {
                        items.iter().map(|x| if let V::Int(c) = x { char::from_u32(*c as u32).unwrap_or('?') } else { '?' }).collect()
                    }
                    x => format!("{f}({})", self.fmt(x, 0)),
                },
                _ => format!("{f}({})", a.iter().map(|x| self.fmt(x, 0)).collect::<Vec<_>>().join(", ")),
            },
        }
    }
}

/// A term shown with `-` in a sum: its negation.
fn negated(x: &V) -> Option<V> {
    match x {
        V::Int(i) if *i < 0 => Some(V::Int(-i)),
        V::Frac(n, d) if *n < 0 => Some(V::Frac(-n, *d)),
        V::Float(m, e) if *m < 0 => Some(V::Float(-m, *e)),
        V::Call(f, a) if f == "neg" => Some(a[0].clone()),
        V::Call(f, a) if f == "*" => negated(&a[0]).map(|c| {
            let mut v = a.clone();
            if is_one(&c) && !is_float(&c) {
                v.remove(0);
                if v.len() == 1 {
                    return v.pop().unwrap();
                }
            } else {
                v[0] = c;
            }
            call("*", v)
        }),
        _ => None,
    }
}

/// Evaluate `expr` and format the result.
pub fn eval_str(expr: &str, m: &Modes) -> Result<String, String> {
    let (s, _) = eval_full(expr, m, false)?;
    Ok(s)
}

/// calc-eval: the result string, and whether it is a number. With
/// `num_only`, a non-numeric result is an error (the `num` separator).
pub fn eval_full(expr: &str, m: &Modes, num_only: bool) -> Result<(String, bool), String> {
    let ast = parse(expr)?;
    let mut c = Calc::new(m.clone());
    let v = c.eval(&ast);
    if let Some(f) = &c.unknown
        && let Some(r) = emacs_calc(expr, m, num_only)
    {
        let _ = f;
        return r;
    }
    let numeric = is_num(&v);
    if num_only && !numeric {
        return Err("Expected a number".into());
    }
    Ok((c.format(&v), numeric))
}

/// The modes as an Elisp plist for `calc-eval`.
fn modes_plist(m: &Modes) -> String {
    let fmt = match m.float_fmt {
        FloatFmt::Float => "float",
        FloatFmt::Fix => "fix",
        FloatFmt::Sci => "sci",
        FloatFmt::Eng => "eng",
    };
    format!(
        "(calc-internal-prec {} calc-float-format ({fmt} {}) calc-angle-mode {} calc-prefer-frac {} calc-symbolic-mode {} calc-date-format (YYYY \"-\" MM \"-\" DD \" \" Www (\" \" hh \":\" mm)) {})",
        m.prec,
        m.figs,
        if m.deg { "deg" } else { "rad" },
        if m.frac { "t" } else { "nil" },
        if m.symbolic { "t" } else { "nil" },
        if m.units { "calc-simplify-mode units" } else { "" }
    )
}

/// Emacs Calc for formulas using functions outside this evaluator.
fn emacs_calc(expr: &str, m: &Modes, num_only: bool) -> Option<Result<(String, bool), String>> {
    if std::env::var_os("FRED_NO_EMACS").is_some() {
        return None;
    }
    let emacs = super::lisp::which("emacs")?;
    let e = format!(
        "(progn (require 'calc) (let ((r (calc-eval (cons {} '{}) {}))) (princ (if (stringp r) (concat \"OK\" r) (format \"ERR%S\" r)))))",
        super::lisp::prin1(&super::lisp::str_(expr)),
        modes_plist(m),
        if num_only { "'num" } else { "nil" }
    );
    let out = std::process::Command::new(emacs).args(["--batch", "-Q", "--eval", &e]).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).into_owned();
    Some(match s.strip_prefix("OK") {
        Some(r) => {
            let numeric = parse(r).is_ok_and(|v| is_num(&v));
            Ok((r.to_owned(), numeric))
        }
        None => Err(s.trim_start_matches("ERR").to_owned()),
    })
}
