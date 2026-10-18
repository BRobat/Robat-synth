use std::cmp::Ordering;
use std::ops::{Add, Div, Mul, Sub};

/// Exact time in cycles. Keeps nested subdivisions free of float drift.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rational {
    num: i64,
    den: i64,
}

fn gcd(mut a: i64, mut b: i64) -> i64 {
    a = a.abs();
    b = b.abs();
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

pub fn lcm(a: i64, b: i64) -> i64 {
    (a / gcd(a, b)) * b
}

impl Rational {
    pub const ZERO: Rational = Rational { num: 0, den: 1 };
    pub const ONE: Rational = Rational { num: 1, den: 1 };

    pub fn new(num: i64, den: i64) -> Self {
        assert!(den != 0, "zero denominator");
        let g = gcd(num, den);
        let s = if den < 0 { -1 } else { 1 };
        Rational { num: s * num / g, den: s * den / g }
    }

    pub fn int(n: i64) -> Self {
        Rational { num: n, den: 1 }
    }

    pub fn num(self) -> i64 {
        self.num
    }

    pub fn den(self) -> i64 {
        self.den
    }

    pub fn floor(self) -> i64 {
        self.num.div_euclid(self.den)
    }

    pub fn ceil(self) -> i64 {
        -(-self.num).div_euclid(self.den)
    }

    pub fn to_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// Parses decimals like `1.5` exactly.
    pub fn from_decimal(s: &str) -> Option<Self> {
        let (int, frac) = s.split_once('.').unwrap_or((s, ""));
        if int.is_empty() && frac.is_empty() {
            return None;
        }
        let digits = format!("{int}{frac}");
        let num: i64 = digits.parse().ok()?;
        let den = 10i64.checked_pow(frac.len() as u32)?;
        Some(Rational::new(num, den))
    }
}

impl Add for Rational {
    type Output = Rational;
    fn add(self, o: Rational) -> Rational {
        let l = lcm(self.den, o.den);
        Rational::new(self.num * (l / self.den) + o.num * (l / o.den), l)
    }
}

impl Sub for Rational {
    type Output = Rational;
    fn sub(self, o: Rational) -> Rational {
        self + Rational { num: -o.num, den: o.den }
    }
}

impl Mul for Rational {
    type Output = Rational;
    fn mul(self, o: Rational) -> Rational {
        let g1 = gcd(self.num, o.den);
        let g2 = gcd(o.num, self.den);
        Rational::new((self.num / g1) * (o.num / g2), (self.den / g2) * (o.den / g1))
    }
}

#[allow(clippy::suspicious_arithmetic_impl)]
impl Div for Rational {
    type Output = Rational;
    fn div(self, o: Rational) -> Rational {
        self * Rational::new(o.den, o.num)
    }
}

impl Ord for Rational {
    fn cmp(&self, o: &Self) -> Ordering {
        (self.num as i128 * o.den as i128).cmp(&(o.num as i128 * self.den as i128))
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_reduces() {
        let a = Rational::new(1, 3) + Rational::new(1, 6);
        assert_eq!(a, Rational::new(1, 2));
        assert_eq!(Rational::new(3, 4) * Rational::new(2, 3), Rational::new(1, 2));
        assert_eq!(Rational::new(-1, 2).floor(), -1);
        assert_eq!(Rational::new(5, 2).ceil(), 3);
        assert_eq!(Rational::from_decimal("1.25"), Some(Rational::new(5, 4)));
    }
}
