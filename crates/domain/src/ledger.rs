//! Double-entry ledger (PLAN.md §13.4).

use crate::ids::*;
use crate::{DomainError, Paise};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccountKind {
    SellerReceivable,
    VendorPayable,
    TransporterPayable,
    PlatformFeeRevenue,
    GstOutput,
    GatewayFees,
    GatewayClearing,
    Refunds,
    Suspense,
}

/// `<kind>` for platform accounts or `<kind>:<userId>` for party accounts.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct AccountId {
    pub kind: AccountKind,
    pub party: Option<UserId>,
}

impl AccountId {
    pub fn platform(kind: AccountKind) -> Self {
        Self { kind, party: None }
    }

    pub fn party(kind: AccountKind, user: UserId) -> Self {
        Self { kind, party: Some(user) }
    }

    pub fn key(&self) -> String {
        let kind = serde_json::to_value(self.kind).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
        match &self.party {
            Some(u) => format!("{kind}:{u}"),
            None => kind,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Side {
    Debit,
    Credit,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Entry {
    pub account: AccountId,
    pub side: Side,
    pub amount: Paise,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TxnKind {
    PaymentCaptured,
    TransferToTransporter,
    PayoutToSeller,
    PayoutFailed,
    DirectPaymentRecorded,
    PlatformFeeInvoiced,
    Refund,
    Adjustment,
}

/// How a Mode A payment is split between parties.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct CaptureSplit {
    pub seller: UserId,
    pub transporter: Option<UserId>,
    pub seller_amount: Paise,
    pub fare: Paise,
    pub fee: Paise,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Transaction {
    /// Razorpay event ID or internal command ID; guarantees exactly-once.
    pub idempotency_key: String,
    pub kind: TxnKind,
    pub deal_id: Option<DealId>,
    pub occurred_at: DateTime<Utc>,
    pub entries: Vec<Entry>,
    pub memo: Option<String>,
}

impl Transaction {
    /// A transaction must have ≥ 2 entries, positive amounts and balance to zero.
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.entries.len() < 2 {
            return Err(DomainError::validation("a transaction needs at least two entries"));
        }
        if self.idempotency_key.is_empty() || self.idempotency_key.contains('#') {
            return Err(DomainError::validation("invalid idempotency key"));
        }
        let mut net: i128 = 0;
        for e in &self.entries {
            if e.amount.0 <= 0 {
                return Err(DomainError::validation("entry amounts must be positive"));
            }
            net += match e.side {
                Side::Debit => e.amount.0 as i128,
                Side::Credit => -(e.amount.0 as i128),
            };
        }
        if net != 0 {
            return Err(DomainError::validation(format!("unbalanced transaction (net {net} paise)")));
        }
        Ok(())
    }

    /// Mode A capture: vendor pays seller amount + fare + fee (PLAN.md §13.2).
    pub fn mode_a_capture(key: String, deal: DealId, split: &CaptureSplit, at: DateTime<Utc>) -> Transaction {
        let CaptureSplit { seller, transporter, seller_amount, fare, fee } = split.clone();
        let total = seller_amount + fare + fee;
        let mut entries = vec![
            Entry { account: AccountId::platform(AccountKind::GatewayClearing), side: Side::Debit, amount: total },
            Entry { account: AccountId::party(AccountKind::SellerReceivable, seller), side: Side::Credit, amount: seller_amount },
        ];
        if fare.0 > 0 {
            let acct = match transporter {
                Some(t) => AccountId::party(AccountKind::TransporterPayable, t),
                None => AccountId::platform(AccountKind::Suspense),
            };
            entries.push(Entry { account: acct, side: Side::Credit, amount: fare });
        }
        if fee.0 > 0 {
            entries.push(Entry { account: AccountId::platform(AccountKind::PlatformFeeRevenue), side: Side::Credit, amount: fee });
        }
        Transaction { idempotency_key: key, kind: TxnKind::PaymentCaptured, deal_id: Some(deal), occurred_at: at, entries, memo: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_balances() {
        let split = CaptureSplit {
            seller: UserId::new(),
            transporter: Some(UserId::new()),
            seller_amount: Paise(2_769_000),
            fare: Paise(30_000),
            fee: Paise(10_000),
        };
        let t = Transaction::mode_a_capture("pay_1".into(), DealId::new(), &split, Utc::now());
        t.validate().unwrap();
        assert_eq!(t.entries.len(), 4);
    }

    #[test]
    fn rejects_unbalanced() {
        let split = CaptureSplit { seller: UserId::new(), transporter: None, seller_amount: Paise(100), fare: Paise(0), fee: Paise(0) };
        let mut t = Transaction::mode_a_capture("pay_2".into(), DealId::new(), &split, Utc::now());
        t.entries[0].amount = Paise(101);
        assert!(t.validate().is_err());
        t.entries[0].amount = Paise(0);
        assert!(t.validate().is_err());
    }

    #[test]
    fn account_keys() {
        assert_eq!(AccountId::platform(AccountKind::PlatformFeeRevenue).key(), "platform_fee_revenue");
        let u = UserId::new();
        assert_eq!(AccountId::party(AccountKind::SellerReceivable, u.clone()).key(), format!("seller_receivable:{u}"));
    }
}
