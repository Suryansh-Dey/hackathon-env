//! Round trip against a real Amazon Verified Permissions policy store that
//! holds the policies in `policies/` (deployed by CDK).
//!
//! Ignored by default and skipped by CI (`--skip live_`); run with
//! `KABADOS_AVP_POLICY_STORE_ID=... cargo test -p kabados-authz -- --ignored live_`.

use kabados_authz::*;

#[tokio::test]
#[ignore = "calls AWS; needs KABADOS_AVP_POLICY_STORE_ID"]
async fn live_verified_permissions_round_trip() {
    let Ok(store) = std::env::var("KABADOS_AVP_POLICY_STORE_ID") else {
        eprintln!("KABADOS_AVP_POLICY_STORE_ID not set; skipping");
        return;
    };
    let conf = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let avp = VerifiedPermissions::new(aws_sdk_verifiedpermissions::Client::new(&conf), store);
    let local = LocalCedar::new().unwrap();

    let seller = UserId::new();
    let vendor = UserId::new();
    let listing = |hazardous| ListingAttrs {
        id: ListingId::new(),
        seller: seller.clone(),
        status: ListingStatus::Published,
        contains_hazardous: hazardous,
        accepted_vendor: None,
        deal_status: None,
        assigned_transporter: None,
        job_status: None,
        drive_admin: None,
    };
    let v = |auth| Principal::Vendor(VendorAttrs { id: vendor.clone(), kyc_approved: true, hazardous_auth_valid: auth, suspended: false });
    let reqs = vec![
        (v(false), Action::MakeOffer, Resource::Listing(listing(false))),
        (v(false), Action::MakeOffer, Resource::Listing(listing(true))),
        (v(true), Action::MakeOffer, Resource::Listing(listing(true))),
        (Principal::Seller(seller.clone()), Action::ViewExactAddress, Resource::Listing(listing(false))),
        (v(true), Action::ViewExactAddress, Resource::Listing(listing(false))),
    ];
    let expected = [true, false, true, true, false];

    for ((p, a, r), want) in reqs.iter().zip(expected) {
        let remote = avp.is_authorized(p, *a, r).await.unwrap();
        assert_eq!(remote.allowed, want, "{} remote {remote:?}", a.as_str());
        assert_eq!(local.decide(p, *a, r).unwrap().allowed, want, "{} local", a.as_str());
    }
    let batch = avp.batch_is_authorized(&reqs).await.unwrap();
    assert_eq!(batch.iter().map(|d| d.allowed).collect::<Vec<_>>(), expected);
}
