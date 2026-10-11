//! Repository behaviour on the in-memory backend (same semantics as DynamoDB,
//! enforced by the conformance suite).

use chrono::Utc;
use kabados_db::memory::MemoryStore;
use kabados_db::repo::*;
use kabados_db::{Store, Table};
use kabados_domain::deal::{Deal, DealStatus};
use kabados_domain::geo::LatLng;
use kabados_domain::ids::*;
use kabados_domain::ladder::LadderLevel;
use kabados_domain::ledger::{CaptureSplit, Transaction};
use kabados_domain::listing::*;
use kabados_domain::offer::*;
use kabados_domain::user::*;
use kabados_domain::{Grams, Paise};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

fn db() -> (Db, Arc<MemoryStore>) {
    let s = Arc::new(MemoryStore::new());
    (Db::new(s.clone()), s)
}

fn profile(phone_hash: &str) -> UserProfile {
    UserProfile {
        id: UserId::new(),
        cognito_sub: format!("sub-{phone_hash}"),
        phone_hash: phone_hash.into(),
        phone_last4: "3210".into(),
        name: Some("Asha".into()),
        roles: BTreeSet::from([Role::Seller]),
        adult_confirmed: true,
        suspended: false,
        honesty: Default::default(),
        created_at: Utc::now(),
    }
}

fn listing(seller: &UserId) -> (Listing, ListingVersion) {
    let p = LatLng { lat: 28.6139, lng: 77.209 };
    let id = ListingId::new();
    let l = Listing {
        id: id.clone(),
        seller_id: seller.clone(),
        status: ListingStatus::Published,
        current_version: 1,
        published_version: Some(1),
        address_id: AddressId::new(),
        location: p,
        approx: ApproxLocation { h3_r7: p.cell_r7().unwrap(), h3_r8: p.cell_r8().unwrap(), center: p, city: "delhi".into() },
        drive_id: None,
        title: "Copper and newspaper".into(),
        contains_hazardous: false,
        categories: vec!["metal.copper".into()],
        total_est_weight_g: Grams(10_000),
        min_ladder: LadderLevel::L0Claimed,
        published_at: Some(Utc::now()),
        first_look_until: None,
        expires_at: None,
        accepted_offer_id: None,
        deal_id: None,
        ai_flagged_for_review: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let v = ListingVersion {
        listing_id: id,
        ver: 1,
        description: Some("old copper wire".into()),
        asking_price: None,
        claim_reason: None,
        pickup: PickupPreferences::default(),
        analysis: AnalysisState {
            status: AnalysisStatus::Done,
            proof_rounds: 0,
            model: None,
            prompt_version: None,
            updated_at: Utc::now(),
        },
        created_at: Utc::now(),
    };
    (l, v)
}

fn offer(l: &Listing, vendor: &UserId) -> Offer {
    Offer {
        id: OfferId::new(),
        listing_id: l.id.clone(),
        listing_version: 1,
        vendor_id: vendor.clone(),
        vendor_name: "Raju Scrap".into(),
        status: OfferStatus::Active,
        lines: vec![],
        adjustments: vec![],
        total_low: Paise(500_000),
        total_high: Paise(600_000),
        covered_weight_low: Grams(9_000),
        covered_weight_high: Grams(11_000),
        distance_m: 2_000,
        valid_until: Utc::now(),
        slot: None,
        pickup_method: PickupMethod::SelfPickup,
        payment_mode: PaymentMode::Direct,
        rule_set_version: Some(1),
        engine_version: "pricing-1.0.0".into(),
        supersedes: None,
        seller_counter: None,
        note: None,
        created_at: Utc::now(),
    }
}

fn deal_from(l: &Listing, o: &Offer) -> Deal {
    Deal {
        id: DealId::new(),
        listing_id: l.id.clone(),
        listing_version: 1,
        offer_id: o.id.clone(),
        seller_id: l.seller_id.clone(),
        vendor_id: o.vendor_id.clone(),
        transporter_id: None,
        job_id: None,
        status: DealStatus::Created,
        payment_mode: o.payment_mode,
        pickup_method: o.pickup_method,
        slot: None,
        rule_set_version: o.rule_set_version,
        engine_version: o.engine_version.clone(),
        agreed_total_low: o.total_low,
        agreed_total_high: o.total_high,
        final_amount: None,
        transporter_fare: None,
        platform_fee: Paise(0),
        payment_order_id: None,
        payment_order_amount: None,
        direct_payment_marked_at: None,
        handover_otp_hash: None,
        handover_at: None,
        delivered_at: None,
        receipt_id: None,
        workflow_execution_arn: None,
        cancel_reason: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[tokio::test]
async fn one_account_per_phone_and_lookup_by_sub() {
    let (db, _) = db();
    let p = db.create_user(profile("h1")).await.unwrap();
    assert!(db.create_user(profile("h1")).await.unwrap_err().is_conflict());
    assert_eq!(db.user_by_sub("sub-h1").await.unwrap().unwrap().data.id, p.data.id);
    assert_eq!(db.user_by_phone_hash("h1").await.unwrap(), Some(p.data.id.clone()));
    let mut rec = db.user(&p.data.id).await.unwrap().unwrap();
    let mut stale = rec.clone();
    rec.data.name = Some("Asha K".into());
    db.save_user(&mut rec).await.unwrap();
    stale.data.name = Some("lost update".into());
    assert!(db.save_user(&mut stale).await.unwrap_err().is_conflict(), "optimistic lock");
}

#[tokio::test]
async fn listing_bundle_and_feed() {
    let (db, _) = db();
    let seller = UserId::new();
    let (l, v) = listing(&seller);
    db.create_listing(l.clone(), v).await.unwrap();
    let item = Item {
        id: ItemId::new(),
        category_id: "metal.copper".into(),
        grade: Some("clean".into()),
        attributes: BTreeMap::new(),
        quantity: 1,
        unit: kabados_domain::taxonomy::Unit::Kg,
        est_weight_g: Some(Grams(10_000)),
        plausible_weight: None,
        hazard: Hazard { is_hazardous: false, class: None, handling: None },
        evidence: vec![],
        source: ItemSource::Ai,
        ai_category_id: Some("metal.copper".into()),
        ai_confidence: Some(0.9),
        ladder: LadderLevel::L1PhotoConsistent,
        note: None,
    };
    db.write_ops(vec![db.item_op(&l.id, 1, &item).unwrap()]).await.unwrap();
    let b = db.listing_bundle(&l.id, None).await.unwrap().unwrap();
    assert_eq!(b.items.len(), 1);
    assert_eq!(b.version.data.ver, 1);
    let (feed, _) = db.feed(&l.approx.h3_r7, None, 10).await.unwrap();
    assert_eq!(feed.len(), 1, "published listings appear in the r7 feed");
    let (mine, _) = db.seller_listings(&seller, None, 10).await.unwrap();
    assert_eq!(mine.len(), 1);
}

#[tokio::test]
async fn accept_offer_is_atomic_and_single_winner() {
    let (db, store) = db();
    let seller = UserId::new();
    let (l, v) = listing(&seller);
    let (mut lrec, _) = db.create_listing(l.clone(), v).await.unwrap();
    let o1 = db.create_offer(offer(&l, &UserId::new()), None, vec![]).await.unwrap();
    let o2 = db.create_offer(offer(&l, &UserId::new()), None, vec![]).await.unwrap();
    assert_eq!(db.offers_for_listing(&l.id).await.unwrap().len(), 2);
    assert_eq!(db.offer(&o2.data.id).await.unwrap().unwrap().data.id, o2.data.id);

    let mut lrec2 = lrec.clone();
    let (mut a, mut b) = (o1.clone(), o2.clone());

    lrec.data.status = ListingStatus::Reserved;
    a.data.status = OfferStatus::Accepted;
    let da = deal_from(&l, &a.data);
    let d = db.accept_offer(&mut lrec, &mut a, da, vec![]).await.unwrap();

    lrec2.data.status = ListingStatus::Reserved;
    b.data.status = OfferStatus::Accepted;
    let dbb = deal_from(&l, &b.data);
    assert!(db.accept_offer(&mut lrec2, &mut b, dbb, vec![]).await.unwrap_err().is_conflict(), "second accept must lose");

    let got = db.deal(&d.data.id).await.unwrap().unwrap();
    assert_eq!(got.data.status, DealStatus::Created);
    let (seller_deals, _) = db.user_deals(&seller, None, None).await.unwrap();
    assert_eq!(seller_deals.len(), 1);
    let outbox: Vec<_> = store.dump(Table::Core).into_iter().filter(|i| i["type"] == "outbox").collect();
    let types: Vec<String> = outbox.iter().map(|i| i["data"]["event"]["type"].as_str().unwrap().to_string()).collect();
    assert!(types.contains(&"OfferAccepted".to_string()) && types.contains(&"DealCreated".to_string()), "{types:?}");
    let (feed, _) = db.feed(&l.approx.h3_r7, None, 10).await.unwrap();
    assert!(feed.is_empty(), "reserved listings leave the sparse feed index");
}

#[tokio::test]
async fn ledger_posts_exactly_once_and_balances() {
    let (db, _) = db();
    let seller = UserId::new();
    let split = CaptureSplit { seller: seller.clone(), transporter: None, seller_amount: Paise(10_000), fare: Paise(0), fee: Paise(500) };
    let t = Transaction::mode_a_capture("pay_X".into(), DealId::new(), &split, Utc::now());
    assert_eq!(db.post_transaction(&t).await.unwrap(), PostOutcome::Posted);
    assert_eq!(db.post_transaction(&t).await.unwrap(), PostOutcome::AlreadyPosted);
    assert_eq!(db.balance(&format!("seller_receivable:{seller}")).await.unwrap(), Paise(10_000));
    assert_eq!(db.balance("platform_fee_revenue").await.unwrap(), Paise(500));
    assert_eq!(db.balance("gateway_clearing").await.unwrap(), Paise(-10_500));
}

#[tokio::test]
async fn idempotency_rate_limits_and_inbox() {
    let (db, _) = db();
    let ttl = chrono::Duration::hours(24);
    assert_eq!(db.idem_begin("api", "k1", "h", ttl).await.unwrap(), IdemState::New);
    assert_eq!(db.idem_begin("api", "k1", "h", ttl).await.unwrap(), IdemState::InProgress);
    assert_eq!(db.idem_begin("api", "k1", "other", ttl).await.unwrap(), IdemState::Mismatch);
    db.idem_complete("api", "k1", "h", 201, serde_json::json!({"ok": true}), ttl).await.unwrap();
    assert!(matches!(db.idem_begin("api", "k1", "h", ttl).await.unwrap(), IdemState::Done { status: 201, .. }));
    assert!(db.first_time("webhook", "evt_1", ttl).await.unwrap());
    assert!(!db.first_time("webhook", "evt_1", ttl).await.unwrap());

    for _ in 0..3 {
        assert!(db.rate_limit("otp", "h1", 600, 3).await.unwrap());
    }
    assert!(!db.rate_limit("otp", "h1", 600, 3).await.unwrap());

    db.set_otp_channel("h1", OtpChannel::Sms).await.unwrap();
    assert_eq!(db.otp_channel("h1").await.unwrap(), Some(OtpChannel::Sms));
    db.inbox_put(
        "dev",
        InboxMessage {
            id: "1".into(),
            channel: "SMS".into(),
            to: "+91******3210".into(),
            template: "otp".into(),
            body: "Code 123456".into(),
            at: Utc::now(),
        },
    )
    .await
    .unwrap();
    assert_eq!(db.inbox("dev", 10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn coverage_rewrite_replaces_stale_entries() {
    let (db, _) = db();
    let v = UserId::new();
    let a: BTreeSet<(String, String)> = [("c1".into(), "metal.copper".into()), ("c2".into(), "metal.copper".into())].into();
    assert_eq!(db.rewrite_coverage(&v, 1, &a).await.unwrap(), (2, 0));
    assert_eq!(db.coverage("c1", "metal.copper").await.unwrap().len(), 1);
    let b: BTreeSet<(String, String)> = [("c2".into(), "metal.copper".into()), ("c3".into(), "paper.newspaper".into())].into();
    assert_eq!(db.rewrite_coverage(&v, 2, &b).await.unwrap(), (2, 1));
    assert!(db.coverage("c1", "metal.copper").await.unwrap().is_empty());
    assert_eq!(db.coverage("c2", "metal.copper").await.unwrap()[0].rule_set_version, 2);
}

#[tokio::test]
async fn audit_log_is_append_only_per_subject() {
    let (db, store) = db();
    db.audit("usr_x", "ops_1", "KYC_APPROVED", serde_json::json!({"note": "ok"})).await.unwrap();
    db.audit("usr_x", "ops_2", "SUSPENDED", serde_json::json!({})).await.unwrap();
    assert_eq!(db.audit_log("usr_x").await.unwrap().len(), 2);
    assert_eq!(store.dump(Table::Audit).len(), 2);
    let _ = store.get(Table::Audit, "x", "y").await.unwrap();
}

#[tokio::test]
async fn user_deals_status_filter_follows_transitions() {
    let (db, _) = db();
    let seller = UserId::new();
    let (l, v) = listing(&seller);
    let (mut lrec, _) = db.create_listing(l.clone(), v).await.unwrap();
    let mut offer = db.create_offer(offer(&l, &UserId::new()), None, vec![]).await.unwrap();
    lrec.data.status = ListingStatus::Reserved;
    offer.data.status = OfferStatus::Accepted;
    let deal = deal_from(&l, &offer.data);
    let mut d = db.accept_offer(&mut lrec, &mut offer, deal, vec![]).await.unwrap();

    // Newly created: CREATED
    let (created, _) = db.user_deals(&seller, Some(DealStatus::Created), None).await.unwrap();
    assert_eq!(created.len(), 1, "created filter");

    // Transition to AWAITING_PAYMENT and re-query.
    d.data.status = DealStatus::AwaitingPayment;
    db.save_deal_with(&mut d, vec![], vec![], vec![]).await.unwrap();
    let (none, _) = db.user_deals(&seller, None, None).await.unwrap();
    assert_eq!(none.len(), 1, "no filter still sees exactly one");
    assert_eq!(none[0].status, DealStatus::AwaitingPayment, "index reflects new status");
    let (old, _) = db.user_deals(&seller, Some(DealStatus::Created), None).await.unwrap();
    let (new, _) = db.user_deals(&seller, Some(DealStatus::AwaitingPayment), None).await.unwrap();
    assert_eq!(old.len(), 0, "stale-status filter must NOT match");
    assert_eq!(new.len(), 1, "new-status filter must match");
}
