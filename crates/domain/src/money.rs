//! Money in paise and weights in grams, both as integers (PLAN.md §12.2).
//!
//! All arithmetic is integer-only with explicit rounding: half away from zero.

use crate::DomainError;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};

/// Indian rupee amount in paise (1 ₹ = 100 paise).
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema,
)]
#[serde(transparent)]
#[schema(value_type = i64)]
pub struct Paise(pub i64);

/// Weight in grams.
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema,
)]
#[serde(transparent)]
#[schema(value_type = i64)]
pub struct Grams(pub i64);

/// Integer division rounding half away from zero. `d` must be positive.
pub fn div_round(n: i128, d: i128) -> i128 {
    debug_assert!(d > 0);
    if n >= 0 { (n + d / 2) / d } else { -((-n + d / 2) / d) }
}

impl Paise {
    pub const ZERO: Paise = Paise(0);

    pub fn from_rupees(r: i64) -> Self {
        Paise(r * 100)
    }

    /// `rate` per kilogram × `w` grams, rounded half away from zero.
    pub fn per_kg(rate: Paise, w: Grams) -> Result<Paise, DomainError> {
        let v = div_round(rate.0 as i128 * w.0 as i128, 1000);
        i64::try_from(v).map(Paise).map_err(|_| DomainError::Overflow)
    }

    /// `rate` per piece × `qty`.
    pub fn per_piece(rate: Paise, qty: u32) -> Result<Paise, DomainError> {
        rate.0.checked_mul(qty as i64).map(Paise).ok_or(DomainError::Overflow)
    }

    /// `self × pct / 100`, rounded half away from zero.
    pub fn pct(self, pct: i64) -> Paise {
        Paise(div_round(self.0 as i128 * pct as i128, 100) as i64)
    }

    pub fn max0(self) -> Paise {
        Paise(self.0.max(0))
    }

    /// en-IN formatting with lakh grouping, e.g. `₹1,23,456.78`.
    pub fn format_inr(self) -> String {
        let neg = self.0 < 0;
        let abs = self.0.unsigned_abs();
        let rupees = abs / 100;
        let paise = abs % 100;
        let grouped = group_indian(rupees);
        let sign = if neg { "-" } else { "" };
        if paise == 0 { format!("{sign}₹{grouped}") } else { format!("{sign}₹{grouped}.{paise:02}") }
    }
}

/// Groups digits the Indian way: last three, then pairs (12,34,567).
pub fn group_indian(n: u64) -> String {
    let s = n.to_string();
    if s.len() <= 3 {
        return s;
    }
    let (head, last3) = s.split_at(s.len() - 3);
    let mut parts: Vec<&str> = Vec::new();
    let mut h = head;
    while h.len() > 2 {
        let (a, b) = h.split_at(h.len() - 2);
        parts.push(b);
        h = a;
    }
    if !h.is_empty() {
        parts.push(h);
    }
    parts.reverse();
    format!("{},{}", parts.join(","), last3)
}

impl fmt::Display for Paise {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.format_inr())
    }
}

impl Add for Paise {
    type Output = Paise;
    fn add(self, o: Paise) -> Paise {
        Paise(self.0 + o.0)
    }
}
impl AddAssign for Paise {
    fn add_assign(&mut self, o: Paise) {
        self.0 += o.0;
    }
}
impl Sub for Paise {
    type Output = Paise;
    fn sub(self, o: Paise) -> Paise {
        Paise(self.0 - o.0)
    }
}
impl SubAssign for Paise {
    fn sub_assign(&mut self, o: Paise) {
        self.0 -= o.0;
    }
}
impl Neg for Paise {
    type Output = Paise;
    fn neg(self) -> Paise {
        Paise(-self.0)
    }
}
impl std::iter::Sum for Paise {
    fn sum<I: Iterator<Item = Paise>>(iter: I) -> Paise {
        Paise(iter.map(|p| p.0).sum())
    }
}

impl Grams {
    pub const ZERO: Grams = Grams(0);

    pub fn from_kg(kg: i64) -> Self {
        Grams(kg * 1000)
    }

    /// `self × pct / 100`, rounded half away from zero.
    pub fn pct(self, pct: i64) -> Grams {
        Grams(div_round(self.0 as i128 * pct as i128, 100) as i64)
    }

    /// Human-readable kilograms with one decimal, e.g. `42.6 kg`.
    pub fn format_kg(self) -> String {
        let tenths = div_round(self.0 as i128, 100) as i64;
        let kg = tenths / 10;
        let frac = (tenths % 10).abs();
        format!("{}.{} kg", group_indian(kg.unsigned_abs()), frac)
    }
}

impl fmt::Display for Grams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.format_kg())
    }
}

impl Add for Grams {
    type Output = Grams;
    fn add(self, o: Grams) -> Grams {
        Grams(self.0 + o.0)
    }
}
impl AddAssign for Grams {
    fn add_assign(&mut self, o: Grams) {
        self.0 += o.0;
    }
}
impl Sub for Grams {
    type Output = Grams;
    fn sub(self, o: Grams) -> Grams {
        Grams(self.0 - o.0)
    }
}
impl std::iter::Sum for Grams {
    fn sum<I: Iterator<Item = Grams>>(iter: I) -> Grams {
        Grams(iter.map(|g| g.0).sum())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn formats_inr_with_lakh_grouping() {
        assert_eq!(Paise(0).format_inr(), "₹0");
        assert_eq!(Paise(99).format_inr(), "₹0.99");
        assert_eq!(Paise(123_456_78).format_inr(), "₹1,23,456.78");
        assert_eq!(Paise(1_00_00_000_00).format_inr(), "₹1,00,00,000");
        assert_eq!(Paise(-1500).format_inr(), "-₹15");
    }

    #[test]
    fn per_kg_rounds_half_away_from_zero() {
        // ₹14.00/kg × 1.5 g = 2.1 paise → 2
        assert_eq!(Paise::per_kg(Paise(1400), Grams(1)).unwrap(), Paise(1));
        // 650 ₹/kg × 42.6 kg = ₹27,690
        assert_eq!(Paise::per_kg(Paise(65000), Grams(42_600)).unwrap(), Paise(2_769_000));
        // exactly half: 1 paise/kg × 500 g = 0.5 → 1
        assert_eq!(Paise::per_kg(Paise(1), Grams(500)).unwrap(), Paise(1));
        assert_eq!(Paise(-250).pct(1), Paise(-3));
    }

    #[test]
    fn formats_kg() {
        assert_eq!(Grams(42_600).format_kg(), "42.6 kg");
        assert_eq!(Grams(1_234_560).format_kg(), "1,234.6 kg");
    }

    proptest! {
        #[test]
        fn per_kg_is_monotonic_in_weight(rate in 0i64..10_000_000, a in 0i64..10_000_000, d in 0i64..1_000_000) {
            let lo = Paise::per_kg(Paise(rate), Grams(a)).unwrap();
            let hi = Paise::per_kg(Paise(rate), Grams(a + d)).unwrap();
            prop_assert!(lo <= hi);
            prop_assert!(lo.0 >= 0);
        }

        #[test]
        fn grouping_round_trips(n in 0u64..10_000_000_000_000) {
            let g = group_indian(n);
            prop_assert_eq!(g.replace(',', "").parse::<u64>().unwrap(), n);
        }
    }
}
