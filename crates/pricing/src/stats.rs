//! Integer order statistics used by suggestions and the Rate Index (PLAN.md §12.6, §12.8).

use kabados_domain::Paise;

/// Quantile by linear interpolation on sorted data, rounded half away from zero.
pub fn quantile(sorted: &[i64], q_permille: u32) -> Option<i64> {
    if sorted.is_empty() {
        return None;
    }
    let n = sorted.len() as i128;
    let pos = (n - 1) * q_permille.min(1000) as i128; // × 1000
    let i = (pos / 1000) as usize;
    let frac = pos % 1000;
    let a = sorted[i] as i128;
    let b = sorted[(i + 1).min(sorted.len() - 1)] as i128;
    Some(kabados_domain::money::div_round(a * 1000 + (b - a) * frac, 1000) as i64)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Summary {
    pub count: u32,
    pub p25: Paise,
    pub median: Paise,
    pub p75: Paise,
}

/// Median and inter-quartile range.
pub fn summarize(values: &[Paise]) -> Option<Summary> {
    let mut v: Vec<i64> = values.iter().map(|p| p.0).collect();
    v.sort_unstable();
    Some(Summary {
        count: v.len() as u32,
        p25: Paise(quantile(&v, 250)?),
        median: Paise(quantile(&v, 500)?),
        p75: Paise(quantile(&v, 750)?),
    })
}

/// Drops values beyond `k_tenths/10 × IQR` outside the quartiles (Rate Index outlier rule).
pub fn without_outliers(values: &[Paise], k_tenths: i64) -> (Vec<Paise>, Vec<Paise>) {
    let Some(s) = summarize(values) else { return (vec![], vec![]) };
    let iqr = s.p75.0 - s.p25.0;
    let lo = s.p25.0 - iqr * k_tenths / 10;
    let hi = s.p75.0 + iqr * k_tenths / 10;
    values.iter().partition(|p| (lo..=hi).contains(&p.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_and_quartiles() {
        let s = summarize(&[Paise(10), Paise(20), Paise(30), Paise(40)]).unwrap();
        assert_eq!((s.p25.0, s.median.0, s.p75.0), (18, 25, 33));
        assert_eq!(summarize(&[Paise(7)]).unwrap().median, Paise(7));
        assert!(summarize(&[]).is_none());
    }

    #[test]
    fn outliers() {
        let v: Vec<Paise> = [100, 101, 102, 103, 104, 5000].into_iter().map(Paise).collect();
        let (kept, dropped) = without_outliers(&v, 15);
        assert_eq!(dropped, vec![Paise(5000)]);
        assert_eq!(kept.len(), 5);
    }
}
