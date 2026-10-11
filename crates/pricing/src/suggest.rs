//! Suggested price for sellers (PLAN.md §12.6).

use crate::stats::{Summary, summarize};
use kabados_domain::Paise;
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SuggestionBasis {
    VendorQuotes,
    BlendedWithRateIndex,
    RateIndexOnly,
    NoPriceData,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Suggestion {
    pub basis: SuggestionBasis,
    pub vendor_count: u32,
    pub low: Option<Paise>,
    pub median: Option<Paise>,
    pub high: Option<Paise>,
}

/// Minimum vendor quotes before the Rate Index is no longer blended in.
pub const MIN_VENDOR_QUOTES: usize = 3;

/// `vendor_mids`: quote midpoints from the top eligible vendors (top-10 by score).
/// `rate_index_value`: the lot valued at Rate Index medians, when published.
pub fn suggest(vendor_mids: &[Paise], rate_index_value: Option<Paise>) -> Suggestion {
    let s: Option<Summary> = summarize(vendor_mids);
    match (s, rate_index_value) {
        (Some(s), _) if vendor_mids.len() >= MIN_VENDOR_QUOTES => Suggestion {
            basis: SuggestionBasis::VendorQuotes,
            vendor_count: s.count,
            low: Some(s.p25),
            median: Some(s.median),
            high: Some(s.p75),
        },
        (Some(s), Some(ri)) => {
            // Weight vendor quotes by count against the index as `MIN_VENDOR_QUOTES - n` virtual quotes.
            let n = vendor_mids.len() as i64;
            let w_ri = MIN_VENDOR_QUOTES as i64 - n;
            let blend = |v: Paise| Paise((v.0 * n + ri.0 * w_ri) / (n + w_ri));
            Suggestion {
                basis: SuggestionBasis::BlendedWithRateIndex,
                vendor_count: s.count,
                low: Some(blend(s.p25).min(blend(s.median))),
                median: Some(blend(s.median)),
                high: Some(blend(s.p75).max(blend(s.median))),
            }
        }
        (Some(s), None) => Suggestion {
            basis: SuggestionBasis::VendorQuotes,
            vendor_count: s.count,
            low: Some(s.p25),
            median: Some(s.median),
            high: Some(s.p75),
        },
        (None, Some(ri)) => Suggestion {
            basis: SuggestionBasis::RateIndexOnly,
            vendor_count: 0,
            low: Some(ri.pct(-10) + ri),
            median: Some(ri),
            high: Some(ri.pct(10) + ri),
        },
        (None, None) => Suggestion { basis: SuggestionBasis::NoPriceData, vendor_count: 0, low: None, median: None, high: None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cases() {
        assert_eq!(suggest(&[], None).basis, SuggestionBasis::NoPriceData);
        assert_eq!(suggest(&[], Some(Paise(1000))).median, Some(Paise(1000)));
        let v = [Paise(900), Paise(1000), Paise(1100)];
        assert_eq!(suggest(&v, Some(Paise(5000))).basis, SuggestionBasis::VendorQuotes);
        let b = suggest(&[Paise(1000)], Some(Paise(2000)));
        assert_eq!(b.basis, SuggestionBasis::BlendedWithRateIndex);
        assert_eq!(b.median, Some(Paise(1666)));
    }
}
