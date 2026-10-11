//! Settlement (PLAN.md §13.1): final amount = Σ(agreed rate × weighed
//! weight) ± agreed modifiers, computed by `kabados-pricing` from scale
//! readings, plus the handover OTP, ledger postings and the Green Receipt.

use crate::error::{ApiError, ApiResult};
use crate::matching::{pickup_ctx, quote_with};
use crate::state::AppState;
use chrono::Utc;
use kabados_db::repo::PostOutcome;
use kabados_db::{Meta, Record, Table, TxOp, keys};
use kabados_domain::deal::*;
use kabados_domain::ids::*;
use kabados_domain::ledger::{AccountId, AccountKind, Entry, Side, Transaction, TxnKind};
use kabados_domain::offer::Rate;
use kabados_domain::user::Sealed;
use kabados_domain::{Grams, Paise};
use kabados_pricing::{Quote, QuoteItem, QuoteMode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Computes the settlement quote from the deal's lines (removed lines excluded).
pub async fn compute(st: &AppState, deal: &Deal, lines: &[DealLine]) -> ApiResult<Quote> {
    let b = st.db.listing_bundle(&deal.listing_id, Some(deal.listing_version)).await?.ok_or(ApiError::NotFound("listing"))?;
    let vendor = st.db.vendor(&deal.vendor_id).await?.ok_or(ApiError::NotFound("vendor"))?;
    let rules = match deal.rule_set_version {
        Some(v) => st.db.rule_set_version(&deal.vendor_id, v).await?,
        None => st.db.latest_rule_set(&deal.vendor_id).await?,
    }
    .ok_or(ApiError::NotFound("rule set"))?
    .data;
    let offer = st.db.offer(&deal.offer_id).await?.ok_or(ApiError::NotFound("offer"))?.data;
    let items: BTreeMap<ItemId, &kabados_domain::listing::Item> = b.items.iter().map(|i| (i.data.id.clone(), &i.data)).collect();
    let mut qitems = Vec::new();
    let mut overrides: BTreeMap<ItemId, Rate> = BTreeMap::new();
    for l in lines.iter().filter(|l| l.status != LineStatus::Removed) {
        let Some(item) = items.get(&l.item_id) else { continue };
        qitems.push(QuoteItem {
            item_id: l.item_id.clone(),
            category_id: l.category_id.clone(),
            grade: l.grade.clone(),
            attributes: item.attributes.clone(),
            quantity: l.quantity,
            est_weight: item.est_weight_g.unwrap_or_default(),
            plausible: item.plausible_weight,
            ladder: item.ladder,
            hazardous: item.hazard.is_hazardous,
            weighed: l.weighed_g,
        });
        overrides.insert(l.item_id.clone(), l.rate);
    }
    let tax = st.taxonomy().await?;
    Ok(quote_with(
        &tax,
        &qitems,
        &pickup_ctx(&b.listing.data, &b.version.data),
        &vendor.data,
        &rules,
        Some(offer.distance_m),
        &overrides,
        QuoteMode::Settlement,
    ))
}

/// Sealed handover code shown only to the seller (PLAN.md §13.1).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HandoverCode {
    pub sealed: Sealed,
}

pub const HANDOVER_SK: &str = "HANDOVER";

/// Generates the handover OTP once goods are paid for; returns the op storing it.
pub async fn issue_handover(st: &AppState, deal: &mut Deal) -> ApiResult<TxOp> {
    let code = kabados_runtime::crypto::numeric_code(4);
    deal.handover_otp_hash = Some(st.hasher.hash(&format!("handover:{}", deal.id), &code));
    let rec = Record::new(Meta::new(keys::deal(&deal.id), HANDOVER_SK, "handover"), HandoverCode { sealed: st.sealer.seal(&code).await? });
    Ok(rec.put_op(Table::Core)?)
}

pub async fn handover_code(st: &AppState, deal: &DealId) -> ApiResult<Option<String>> {
    match st.db.get_rec::<HandoverCode>(Table::Core, &keys::deal(deal), HANDOVER_SK).await? {
        Some(r) => Ok(Some(st.sealer.open(&r.data.sealed).await?)),
        None => Ok(None),
    }
}

pub fn verify_handover(st: &AppState, deal: &Deal, code: &str) -> bool {
    deal.handover_otp_hash.as_ref().is_some_and(|h| st.hasher.verify(&format!("handover:{}", deal.id), code.trim(), h))
}

/// Ledger postings at settlement. Mode B: the platform fee becomes receivable
/// from the vendor (billed separately). Mode A postings happen on capture.
pub async fn post_ledger(st: &AppState, deal: &Deal) -> ApiResult<()> {
    if deal.payment_mode == kabados_domain::offer::PaymentMode::Direct && deal.platform_fee.0 > 0 {
        let t = Transaction {
            idempotency_key: format!("fee-{}", deal.id),
            kind: TxnKind::PlatformFeeInvoiced,
            deal_id: Some(deal.id.clone()),
            occurred_at: Utc::now(),
            entries: vec![
                Entry {
                    account: AccountId::party(AccountKind::VendorPayable, deal.vendor_id.clone()),
                    side: Side::Debit,
                    amount: deal.platform_fee,
                },
                Entry { account: AccountId::platform(AccountKind::PlatformFeeRevenue), side: Side::Credit, amount: deal.platform_fee },
            ],
            memo: Some("Platform fee for a direct-payment deal".into()),
        };
        let _: PostOutcome = st.db.post_transaction(&t).await?;
    }
    Ok(())
}

/// Builds the Green Receipt from final lines (weights from scale readings only).
pub async fn receipt(st: &AppState, deal: &Deal, lines: &[DealLine]) -> ApiResult<GreenReceipt> {
    let tax = st.taxonomy().await?;
    let listing = st.db.listing(&deal.listing_id).await?.ok_or(ApiError::NotFound("listing"))?;
    let vendor = st.db.vendor(&deal.vendor_id).await?.ok_or(ApiError::NotFound("vendor"))?;
    let mut out = Vec::new();
    let mut co2 = 0i64;
    for l in lines.iter().filter(|l| l.status != LineStatus::Removed) {
        let w = l.weighed_g.unwrap_or_default();
        let cat = tax.get(&l.category_id);
        co2 += w.0 * cat.map(|c| c.emission_factor_g_per_kg).unwrap_or(0) / 1000;
        out.push(ReceiptLine {
            category_id: l.category_id.clone(),
            category_name: cat.map(|c| c.name.clone()).unwrap_or_else(|| l.category_id.clone()),
            weight: w,
            hazardous: cat.is_some_and(|c| c.hazardous),
        });
    }
    Ok(GreenReceipt {
        id: ReceiptId::new(),
        deal_id: deal.id.clone(),
        issued_at: Utc::now(),
        city: listing.data.approx.city.clone(),
        vendor_name: vendor.data.business_name.clone(),
        vendor_authorized_for_hazardous: vendor.data.hazardous_auth_valid(Utc::now()),
        total_weight: out.iter().map(|l| l.weight).sum::<Grams>(),
        total_amount: deal.final_amount.unwrap_or_default(),
        lines: out,
        co2e_avoided_g: co2,
        emission_factor_source: tax.emission_factor_source.clone(),
        disclaimer: RECEIPT_DISCLAIMER.into(),
    })
}

/// Signature for public receipt verification links (PLAN.md §10.2).
pub fn receipt_sig(st: &AppState, id: &ReceiptId) -> String {
    st.hasher.hash("receipt", id.as_str())[..16].to_owned()
}

pub fn amount_of(q: &Quote, item: &ItemId) -> Option<Paise> {
    q.lines.iter().find(|l| &l.item_id == item).map(|l| l.value_low)
}
