//! KYC verification port (PLAN.md §7.1, §8.3). Verification is currently
//! skipped by product decision: submissions are approved after basic format
//! checks, so onboarding is never blocked.

use async_trait::async_trait;
use chrono::Utc;
use kabados_domain::user::{DocumentKind, KycCheck, KycCheckOutcome};
use regex::Regex;
use std::sync::OnceLock;

#[async_trait]
pub trait KycPort: Send + Sync {
    async fn verify(&self, kind: DocumentKind, number: Option<&str>, name: Option<&str>) -> anyhow::Result<KycCheck>;
}

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).expect("valid regex"))
}

pub fn is_pan(s: &str) -> bool {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"^[A-Z]{5}[0-9]{4}[A-Z]$").is_match(s)
}

/// GSTIN with its mod-36 check character.
pub fn is_gstin(s: &str) -> bool {
    static R: OnceLock<Regex> = OnceLock::new();
    if !re(&R, r"^[0-9]{2}[A-Z]{5}[0-9]{4}[A-Z][1-9A-Z]Z[0-9A-Z]$").is_match(s) {
        return false;
    }
    const CHARS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut sum = 0u32;
    for (i, c) in s.bytes().take(14).enumerate() {
        let v = CHARS.iter().position(|&x| x == c).unwrap_or(0) as u32;
        let p = v * if i % 2 == 0 { 1 } else { 2 };
        sum += p / 36 + p % 36;
    }
    let check = (36 - sum % 36) % 36;
    s.as_bytes()[14] == CHARS[check as usize]
}

pub fn is_driving_licence(s: &str) -> bool {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"^[A-Z]{2}[0-9]{2}[ -]?[0-9]{4}[0-9]{7}$").is_match(s)
}

pub fn is_vehicle_rc(s: &str) -> bool {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"^[A-Z]{2}[0-9]{1,2}[A-Z]{0,3}[0-9]{4}$").is_match(&s.replace([' ', '-'], ""))
}

/// Skips government-record verification: approves, noting any format issue.
pub struct SkipKyc;

#[async_trait]
impl KycPort for SkipKyc {
    async fn verify(&self, kind: DocumentKind, number: Option<&str>, _name: Option<&str>) -> anyhow::Result<KycCheck> {
        let n = number.map(|s| s.trim().to_ascii_uppercase());
        let format_ok = match (kind, n.as_deref()) {
            (DocumentKind::Pan, Some(v)) => is_pan(v),
            (DocumentKind::Gstin, Some(v)) => is_gstin(v),
            (DocumentKind::DrivingLicence, Some(v)) => is_driving_licence(v),
            (DocumentKind::VehicleRc, Some(v)) => is_vehicle_rc(v),
            _ => true,
        };
        Ok(KycCheck {
            outcome: KycCheckOutcome::Verified,
            provider: "skip".into(),
            detail: (!format_ok).then(|| "number format looks unusual; verification skipped".into()),
            at: Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert!(is_pan("ABCDE1234F"));
        assert!(!is_pan("ABCD1234F"));
        assert!(is_gstin("27AAPFU0939F1ZV"), "known-valid sample GSTIN");
        assert!(!is_gstin("27AAPFU0939F1ZX"));
        assert!(is_driving_licence("DL04 20110012345"));
        assert!(is_vehicle_rc("DL 3C AB 1234"));
    }

    #[tokio::test]
    async fn skip_always_approves() {
        let c = SkipKyc.verify(DocumentKind::Pan, Some("bad"), None).await.unwrap();
        assert_eq!(c.outcome, KycCheckOutcome::Verified);
        assert!(c.detail.is_some());
    }
}
