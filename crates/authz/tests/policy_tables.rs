//! Allow/deny tables over every role × action × state (PLAN.md §18.3).
//!
//! Expectations are derived from the domain state machines wherever possible
//! so policies cannot drift from `kabados-domain`.

use kabados_authz::*;
use kabados_domain::state::StateMachine;

const LISTING: [ListingStatus; 12] = [
    ListingStatus::Draft,
    ListingStatus::Analyzing,
    ListingStatus::NeedsInput,
    ListingStatus::Ready,
    ListingStatus::Published,
    ListingStatus::Reserved,
    ListingStatus::Completed,
    ListingStatus::PartiallyCompleted,
    ListingStatus::Cancelled,
    ListingStatus::Expired,
    ListingStatus::Withdrawn,
    ListingStatus::Blocked,
];

const DEAL: [DealStatus; 19] = [
    DealStatus::Created,
    DealStatus::PickupScheduled,
    DealStatus::JobAssigned,
    DealStatus::AtPickup,
    DealStatus::Weighed,
    DealStatus::LineRegraded,
    DealStatus::SellerConfirmed,
    DealStatus::AwaitingPayment,
    DealStatus::Paid,
    DealStatus::PaidDirect,
    DealStatus::HandedOver,
    DealStatus::InTransit,
    DealStatus::Delivered,
    DealStatus::Settled,
    DealStatus::Closed,
    DealStatus::SellerNoShow,
    DealStatus::PickupNoShow,
    DealStatus::Disputed,
    DealStatus::Cancelled,
];

const JOB: [JobStatus; 12] = [
    JobStatus::Offered,
    JobStatus::Accepted,
    JobStatus::Declined,
    JobStatus::TimedOut,
    JobStatus::EnRoute,
    JobStatus::ArrivedPickup,
    JobStatus::Loaded,
    JobStatus::InTransit,
    JobStatus::ArrivedYard,
    JobStatus::Delivered,
    JobStatus::PaidOut,
    JobStatus::Cancelled,
];

const OFFER: [OfferStatus; 6] = [
    OfferStatus::Active,
    OfferStatus::Accepted,
    OfferStatus::Rejected,
    OfferStatus::Withdrawn,
    OfferStatus::Expired,
    OfferStatus::Superseded,
];

const DRIVE: [DriveStatus; 4] = [DriveStatus::Open, DriveStatus::Closed, DriveStatus::Completed, DriveStatus::Cancelled];

/// Fixed cast of users.
struct World {
    seller: UserId,
    other_seller: UserId,
    vendor: UserId,
    other_vendor: UserId,
    transporter: UserId,
    other_transporter: UserId,
    admin: UserId,
    other_admin: UserId,
    engine: LocalCedar,
}

impl World {
    fn new() -> Self {
        World {
            seller: UserId::new(),
            other_seller: UserId::new(),
            vendor: UserId::new(),
            other_vendor: UserId::new(),
            transporter: UserId::new(),
            other_transporter: UserId::new(),
            admin: UserId::new(),
            other_admin: UserId::new(),
            engine: LocalCedar::new().expect("policies load"),
        }
    }

    fn vendor_p(&self, id: &UserId) -> Principal {
        Principal::Vendor(VendorAttrs { id: id.clone(), kyc_approved: true, hazardous_auth_valid: false, suspended: false })
    }

    fn transporter_p(&self, id: &UserId) -> Principal {
        Principal::Transporter(TransporterAttrs { id: id.clone(), kyc_approved: true, suspended: false })
    }

    fn allowed(&self, p: &Principal, a: Action, r: &Resource) -> bool {
        let d = self.engine.decide(p, a, r).unwrap_or_else(|e| panic!("{} on {:?}: {e}", a.as_str(), r));
        assert!(!d.allowed || d.errors.is_empty(), "allowed with errors: {d:?}");
        d.allowed
    }

    fn listing(&self, status: ListingStatus) -> ListingAttrs {
        ListingAttrs {
            id: ListingId::new(),
            seller: self.seller.clone(),
            status,
            contains_hazardous: false,
            accepted_vendor: None,
            deal_status: None,
            assigned_transporter: None,
            job_status: None,
            drive_admin: Some(self.admin.clone()),
        }
    }

    fn deal(&self, status: DealStatus) -> DealAttrs {
        DealAttrs {
            id: DealId::new(),
            seller: self.seller.clone(),
            vendor: self.vendor.clone(),
            transporter: Some(self.transporter.clone()),
            status,
        }
    }

    /// Every principal in the cast (including flag variants).
    fn everyone(&self) -> Vec<(&'static str, Principal)> {
        vec![
            ("seller", Principal::Seller(self.seller.clone())),
            ("other_seller", Principal::Seller(self.other_seller.clone())),
            ("vendor", self.vendor_p(&self.vendor)),
            ("other_vendor", self.vendor_p(&self.other_vendor)),
            ("transporter", self.transporter_p(&self.transporter)),
            ("other_transporter", self.transporter_p(&self.other_transporter)),
            ("admin", Principal::SocietyAdmin(self.admin.clone())),
            ("other_admin", Principal::SocietyAdmin(self.other_admin.clone())),
            ("ops", Principal::Ops { id: "ops-1".into(), mfa: true }),
            ("ops_no_mfa", Principal::Ops { id: "ops-2".into(), mfa: false }),
            // The seller's own user acting as a vendor (one account, several roles).
            ("seller_as_vendor", self.vendor_p(&self.seller)),
        ]
    }
}

fn check(w: &World, label: &str, p: &Principal, a: Action, r: &Resource, expect: bool) {
    assert_eq!(w.allowed(p, a, r), expect, "{label}: {} expected {}", a.as_str(), if expect { "ALLOW" } else { "DENY" });
}

#[test]
fn exact_address_only_for_owner_active_deal_vendor_and_active_job_transporter() {
    let w = World::new();
    let mut checked = 0;
    for ds in DEAL {
        for js in JOB {
            let mut l = w.listing(ListingStatus::Reserved);
            l.accepted_vendor = Some(w.vendor.clone());
            l.deal_status = Some(ds);
            l.assigned_transporter = Some(w.transporter.clone());
            l.job_status = Some(js);
            let r = Resource::Listing(l);
            for (who, p) in w.everyone() {
                let expect = match who {
                    "seller" => true,
                    "vendor" => DealStatus::EXACT_ADDRESS_STATES.contains(&ds),
                    "transporter" => JobStatus::EXACT_ADDRESS_STATES.contains(&js),
                    _ => false,
                };
                check(&w, &format!("{who} deal={ds:?} job={js:?}"), &p, Action::ViewExactAddress, &r, expect);
                checked += 1;
            }
        }
    }
    assert_eq!(checked, DEAL.len() * JOB.len() * w.everyone().len());

    // Without an accepted vendor / assignment nobody but the seller sees it.
    for ls in LISTING {
        let r = Resource::Listing(w.listing(ls));
        for (who, p) in w.everyone() {
            check(&w, &format!("{who} {ls:?}"), &p, Action::ViewExactAddress, &r, who == "seller");
        }
    }
    // A suspended or unapproved assigned transporter never sees it.
    for (kyc, suspended) in [(false, false), (true, true)] {
        let mut l = w.listing(ListingStatus::Reserved);
        l.assigned_transporter = Some(w.transporter.clone());
        l.job_status = Some(JobStatus::EnRoute);
        let t = Principal::Transporter(TransporterAttrs { id: w.transporter.clone(), kyc_approved: kyc, suspended });
        check(&w, "unverified transporter", &t, Action::ViewExactAddress, &Resource::Listing(l), false);
    }
}

#[test]
fn make_offer_matrix() {
    let w = World::new();
    for ls in LISTING {
        for hazardous in [false, true] {
            for bits in 0..8u8 {
                let (kyc, auth, suspended) = (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0);
                for own in [false, true] {
                    let mut l = w.listing(ls);
                    l.contains_hazardous = hazardous;
                    let id = if own { w.seller.clone() } else { w.vendor.clone() };
                    let p = Principal::Vendor(VendorAttrs { id, kyc_approved: kyc, hazardous_auth_valid: auth, suspended });
                    let expect = ls == ListingStatus::Published && kyc && !suspended && (!hazardous || auth) && !own;
                    check(
                        &w,
                        &format!("{ls:?} haz={hazardous} kyc={kyc} auth={auth} susp={suspended} own={own}"),
                        &p,
                        Action::MakeOffer,
                        &Resource::Listing(l),
                        expect,
                    );
                }
            }
        }
    }
    // Only vendors make offers.
    let r = Resource::Listing(w.listing(ListingStatus::Published));
    for (who, p) in w.everyone() {
        if !matches!(p, Principal::Vendor(_)) {
            check(&w, who, &p, Action::MakeOffer, &r, false);
        }
    }
}

#[test]
fn vendor_listing_visibility() {
    let w = World::new();
    for ls in LISTING {
        for hazardous in [false, true] {
            for bits in 0..8u8 {
                let (kyc, auth, suspended) = (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0);
                for accepted in [false, true] {
                    let mut l = w.listing(ls);
                    l.contains_hazardous = hazardous;
                    if accepted {
                        l.accepted_vendor = Some(w.vendor.clone());
                    }
                    let p =
                        Principal::Vendor(VendorAttrs { id: w.vendor.clone(), kyc_approved: kyc, hazardous_auth_valid: auth, suspended });
                    let browse = kyc && !suspended && ls == ListingStatus::Published;
                    let expect = (browse || accepted) && (!hazardous || auth);
                    check(
                        &w,
                        &format!("{ls:?} haz={hazardous} kyc={kyc} auth={auth} susp={suspended} accepted={accepted}"),
                        &p,
                        Action::ViewListing,
                        &Resource::Listing(l),
                        expect,
                    );
                }
            }
        }
    }
}

#[test]
fn seller_listing_management_follows_domain_state_machine() {
    let w = World::new();
    for ls in LISTING {
        let r = Resource::Listing(w.listing(ls));
        let owner = Principal::Seller(w.seller.clone());
        let other = Principal::Seller(w.other_seller.clone());
        check(&w, &format!("{ls:?}"), &owner, Action::ViewListing, &r, true);
        check(&w, &format!("{ls:?}"), &owner, Action::ViewOffers, &r, true);
        check(&w, &format!("{ls:?}"), &owner, Action::EditListing, &r, ls.is_editable());
        check(&w, &format!("{ls:?}"), &owner, Action::PublishListing, &r, ls.can_transition_to(ListingStatus::Published));
        check(&w, &format!("{ls:?}"), &owner, Action::WithdrawListing, &r, ls.can_transition_to(ListingStatus::Withdrawn));
        for a in [Action::ViewListing, Action::ViewOffers, Action::EditListing, Action::PublishListing, Action::WithdrawListing] {
            check(&w, &format!("other seller {ls:?}"), &other, a, &r, false);
        }
        // Drive admins see pooled listings, never the address; other admins nothing.
        check(&w, "drive admin", &Principal::SocietyAdmin(w.admin.clone()), Action::ViewListing, &r, true);
        check(&w, "other admin", &Principal::SocietyAdmin(w.other_admin.clone()), Action::ViewListing, &r, false);
        // Vendors never see the full offer list.
        check(&w, "vendor offers", &w.vendor_p(&w.vendor), Action::ViewOffers, &r, false);
    }
}

#[test]
fn offer_responses() {
    let w = World::new();
    for ls in LISTING {
        for os in OFFER {
            let r = Resource::Offer(OfferAttrs { id: OfferId::new(), listing: w.listing(ls), vendor: w.vendor.clone(), status: os });
            let live = ls == ListingStatus::Published && os == OfferStatus::Active;
            for a in [Action::AcceptOffer, Action::CounterOffer, Action::RejectOffer] {
                check(&w, &format!("seller {ls:?} {os:?}"), &Principal::Seller(w.seller.clone()), a, &r, live);
                check(&w, "other seller", &Principal::Seller(w.other_seller.clone()), a, &r, false);
                check(&w, "vendor", &w.vendor_p(&w.vendor), a, &r, false);
            }
            check(&w, &format!("vendor withdraw {os:?}"), &w.vendor_p(&w.vendor), Action::WithdrawOffer, &r, os == OfferStatus::Active);
            check(&w, "other vendor withdraw", &w.vendor_p(&w.other_vendor), Action::WithdrawOffer, &r, false);
            check(&w, "seller withdraw", &Principal::Seller(w.seller.clone()), Action::WithdrawOffer, &r, false);
        }
    }
}

/// Expected deal permissions for a participant role in a given status.
fn deal_expect(role: &str, a: Action, s: DealStatus) -> bool {
    use DealStatus::*;
    let seller = role == "seller";
    let vendor = role == "vendor";
    let transporter = role == "transporter";
    let participant = seller || vendor || transporter;
    match a {
        Action::ViewDeal => participant,
        Action::ViewReceipt => seller || vendor,
        Action::RecordWeighing | Action::RegradeLine => (vendor || transporter) && matches!(s, AtPickup | Weighed | LineRegraded),
        Action::ConfirmWeights => seller && s == Weighed && s.can_transition_to(SellerConfirmed),
        Action::RequestPayment => vendor && s == SellerConfirmed && s.can_transition_to(AwaitingPayment),
        Action::MarkPaidDirect => vendor && s == SellerConfirmed && s.can_transition_to(PaidDirect),
        Action::ConfirmReceived => seller && s == SellerConfirmed,
        // The seller never hands over goods unpaid.
        Action::Handover => (vendor || transporter) && s.can_transition_to(HandedOver),
        Action::ConfirmDelivery => vendor && s.can_transition_to(Delivered) && s != Disputed,
        Action::CancelDeal => (seller || vendor) && s.is_pre_pickup() && s.can_transition_to(Cancelled),
        Action::OpenDispute => participant && s.is_open(),
        Action::RateDeal => participant && matches!(s, Delivered | Settled | Closed),
        Action::StartCall => participant && !matches!(s, Settled | Closed | Cancelled),
        Action::AssignTransporter => vendor && matches!(s, Created | PickupScheduled),
        _ => unreachable!(),
    }
}

const DEAL_ACTIONS: [Action; 15] = [
    Action::ViewDeal,
    Action::ViewReceipt,
    Action::RecordWeighing,
    Action::RegradeLine,
    Action::ConfirmWeights,
    Action::RequestPayment,
    Action::MarkPaidDirect,
    Action::ConfirmReceived,
    Action::Handover,
    Action::ConfirmDelivery,
    Action::CancelDeal,
    Action::OpenDispute,
    Action::RateDeal,
    Action::StartCall,
    Action::AssignTransporter,
];

#[test]
fn deal_matrix_every_role_action_and_status() {
    let w = World::new();
    let mut n = 0;
    for s in DEAL {
        let r = Resource::Deal(w.deal(s));
        for (who, p) in w.everyone() {
            for a in DEAL_ACTIONS {
                let expect = match (who, &p) {
                    ("ops", _) => a == Action::ViewDeal,
                    _ => deal_expect(who, a, s),
                };
                check(&w, &format!("{who} {s:?}"), &p, a, &r, expect);
                n += 1;
            }
        }
    }
    assert_eq!(n, DEAL.len() * DEAL_ACTIONS.len() * w.everyone().len());
}

#[test]
fn deal_without_transporter_and_unverified_transporters() {
    let w = World::new();
    for s in DEAL {
        let mut d = w.deal(s);
        d.transporter = None;
        let r = Resource::Deal(d);
        for a in DEAL_ACTIONS {
            check(&w, &format!("unassigned transporter {s:?}"), &w.transporter_p(&w.transporter), a, &r, false);
        }
        let r = Resource::Deal(w.deal(s));
        for (kyc, suspended) in [(false, false), (true, true), (false, true)] {
            let t = Principal::Transporter(TransporterAttrs { id: w.transporter.clone(), kyc_approved: kyc, suspended });
            for a in DEAL_ACTIONS {
                check(&w, &format!("unverified transporter {s:?}"), &t, a, &r, false);
            }
        }
    }
}

#[test]
fn job_matrix() {
    let w = World::new();
    let job_actions = [Action::ViewJob, Action::AcceptJob, Action::DeclineJob, Action::UpdateJob, Action::PostJobPosition];
    for js in JOB {
        // Offered to `transporter`, assigned to `other_transporter`.
        let r = Resource::Job(JobAttrs {
            id: JobId::new(),
            deal: w.deal(DealStatus::JobAssigned),
            transporter: Some(w.other_transporter.clone()),
            offered_to: vec![w.transporter.clone()],
            status: js,
        });
        let offered = js == JobStatus::Offered;
        for a in job_actions {
            let label = format!("{js:?}");
            let offered_expect = matches!(a, Action::ViewJob | Action::AcceptJob | Action::DeclineJob) && offered;
            check(&w, &format!("offered transporter {label}"), &w.transporter_p(&w.transporter), a, &r, offered_expect);
            let assigned_expect = match a {
                Action::ViewJob => true,
                Action::UpdateJob => js.shares_location() || js == JobStatus::Delivered,
                Action::PostJobPosition => js.shares_location(),
                _ => false,
            };
            check(&w, &format!("assigned transporter {label}"), &w.transporter_p(&w.other_transporter), a, &r, assigned_expect);
            let vendor_expect = match a {
                Action::ViewJob => true,
                Action::UpdateJob => matches!(js, JobStatus::Offered | JobStatus::Accepted | JobStatus::EnRoute | JobStatus::ArrivedPickup),
                _ => false,
            };
            check(&w, &format!("deal vendor {label}"), &w.vendor_p(&w.vendor), a, &r, vendor_expect);
            check(
                &w,
                &format!("deal seller {label}"),
                &Principal::Seller(w.seller.clone()),
                a,
                &r,
                a == Action::ViewJob && js.shares_location(),
            );
            check(&w, &format!("other vendor {label}"), &w.vendor_p(&w.other_vendor), a, &r, false);
            check(&w, &format!("other seller {label}"), &Principal::Seller(w.other_seller.clone()), a, &r, false);
            let unverified = Principal::Transporter(TransporterAttrs { id: w.transporter.clone(), kyc_approved: false, suspended: false });
            check(&w, &format!("unverified offered transporter {label}"), &unverified, a, &r, false);
        }
    }
}

#[test]
fn thread_matrix() {
    let w = World::new();
    for ls in LISTING {
        for deal_open in [false, true] {
            let r = Resource::Thread(ThreadAttrs {
                id: ThreadId::new(),
                seller: w.seller.clone(),
                vendor: w.vendor.clone(),
                listing_status: ls,
                deal_open,
            });
            let can_post = deal_open || !matches!(ls, ListingStatus::Withdrawn | ListingStatus::Blocked);
            let label = format!("{ls:?} deal_open={deal_open}");
            check(&w, &label, &Principal::Seller(w.seller.clone()), Action::ViewThread, &r, true);
            check(&w, &label, &w.vendor_p(&w.vendor), Action::ViewThread, &r, true);
            check(&w, &label, &Principal::Seller(w.seller.clone()), Action::PostMessage, &r, can_post);
            check(&w, &label, &w.vendor_p(&w.vendor), Action::PostMessage, &r, can_post);
            let suspended =
                Principal::Vendor(VendorAttrs { id: w.vendor.clone(), kyc_approved: true, hazardous_auth_valid: false, suspended: true });
            check(&w, &label, &suspended, Action::ViewThread, &r, true);
            check(&w, &label, &suspended, Action::PostMessage, &r, false);
            for (who, p) in [("other seller", Principal::Seller(w.other_seller.clone())), ("other vendor", w.vendor_p(&w.other_vendor))] {
                check(&w, who, &p, Action::ViewThread, &r, false);
                check(&w, who, &p, Action::PostMessage, &r, false);
            }
        }
    }
}

#[test]
fn drive_matrix() {
    let w = World::new();
    for ds in DRIVE {
        let r = Resource::Drive(DriveAttrs { id: DriveId::new(), admin: w.admin.clone(), status: ds });
        for (who, p) in w.everyone() {
            let label = format!("{who} {ds:?}");
            check(&w, &label, &p, Action::ManageDrive, &r, who == "admin");
            check(&w, &label, &p, Action::ViewDriveReport, &r, who == "admin");
            let join = matches!(p, Principal::Seller(_)) && ds == DriveStatus::Open;
            check(&w, &label, &p, Action::JoinDrive, &r, join);
        }
    }
}

const OPS_ACTIONS: [Action; 7] = [
    Action::OpsReviewKyc,
    Action::OpsReviewAi,
    Action::OpsResolveDispute,
    Action::OpsManageTaxonomy,
    Action::OpsManageConfig,
    Action::OpsViewFraud,
    Action::OpsPublishRateIndex,
];

#[test]
fn ops_require_mfa_and_never_see_addresses() {
    let w = World::new();
    let resources =
        [Resource::Platform, Resource::Listing(w.listing(ListingStatus::Published)), Resource::Deal(w.deal(DealStatus::Disputed))];
    for r in &resources {
        for (who, p) in w.everyone() {
            for a in OPS_ACTIONS {
                check(&w, who, &p, a, r, who == "ops");
            }
        }
    }
    let ops = Principal::Ops { id: "ops-1".into(), mfa: true };
    let no_mfa = Principal::Ops { id: "ops-2".into(), mfa: false };
    let l = Resource::Listing(w.listing(ListingStatus::Published));
    for a in [Action::ViewListing, Action::ViewOffers] {
        check(&w, "ops", &ops, a, &l, true);
        check(&w, "ops no mfa", &no_mfa, a, &l, false);
    }
    check(&w, "ops", &ops, Action::ViewExactAddress, &l, false);
    check(&w, "ops deal", &ops, Action::ViewDeal, &resources[2], true);
    check(&w, "ops no mfa deal", &no_mfa, Action::ViewDeal, &resources[2], false);
}

#[test]
fn schema_inapplicable_requests_are_denied_with_a_reason() {
    let w = World::new();
    let offer = Resource::Offer(OfferAttrs {
        id: OfferId::new(),
        listing: w.listing(ListingStatus::Published),
        vendor: w.vendor.clone(),
        status: OfferStatus::Active,
    });
    let d = w.engine.decide(&w.transporter_p(&w.transporter), Action::AcceptOffer, &offer).unwrap();
    assert!(!d.allowed);
    assert!(!d.errors.is_empty(), "request-validation reason expected");
    // Action applied to the wrong resource type.
    let d = w.engine.decide(&Principal::Seller(w.seller.clone()), Action::AcceptOffer, &Resource::Platform).unwrap();
    assert!(!d.allowed && !d.errors.is_empty());
}

#[test]
fn every_action_has_an_allowing_case() {
    let w = World::new();
    let seller = Principal::Seller(w.seller.clone());
    let vendor = w.vendor_p(&w.vendor);
    let mut auth_vendor = vendor.clone();
    if let Principal::Vendor(v) = &mut auth_vendor {
        v.hazardous_auth_valid = true;
    }
    let transporter = w.transporter_p(&w.transporter);
    let ops = Principal::Ops { id: "ops".into(), mfa: true };
    let admin = Principal::SocietyAdmin(w.admin.clone());
    let published = Resource::Listing(w.listing(ListingStatus::Published));
    let ready = Resource::Listing(w.listing(ListingStatus::Ready));
    let offer = Resource::Offer(OfferAttrs {
        id: OfferId::new(),
        listing: w.listing(ListingStatus::Published),
        vendor: w.vendor.clone(),
        status: OfferStatus::Active,
    });
    let deal = |s| Resource::Deal(w.deal(s));
    let job = |s, assigned: bool| {
        Resource::Job(JobAttrs {
            id: JobId::new(),
            deal: w.deal(DealStatus::JobAssigned),
            transporter: assigned.then(|| w.transporter.clone()),
            offered_to: vec![w.transporter.clone()],
            status: s,
        })
    };
    let thread = Resource::Thread(ThreadAttrs {
        id: ThreadId::new(),
        seller: w.seller.clone(),
        vendor: w.vendor.clone(),
        listing_status: ListingStatus::Published,
        deal_open: false,
    });
    let drive = Resource::Drive(DriveAttrs { id: DriveId::new(), admin: w.admin.clone(), status: DriveStatus::Open });

    let cases: Vec<(Principal, Action, Resource)> = vec![
        (seller.clone(), Action::ViewListing, published.clone()),
        (seller.clone(), Action::EditListing, ready.clone()),
        (seller.clone(), Action::PublishListing, ready.clone()),
        (seller.clone(), Action::WithdrawListing, published.clone()),
        (seller.clone(), Action::ViewExactAddress, published.clone()),
        (auth_vendor.clone(), Action::MakeOffer, published.clone()),
        (seller.clone(), Action::ViewOffers, published.clone()),
        (seller.clone(), Action::AcceptOffer, offer.clone()),
        (seller.clone(), Action::CounterOffer, offer.clone()),
        (seller.clone(), Action::RejectOffer, offer.clone()),
        (vendor.clone(), Action::WithdrawOffer, offer.clone()),
        (vendor.clone(), Action::ViewDeal, deal(DealStatus::Created)),
        (transporter.clone(), Action::RecordWeighing, deal(DealStatus::AtPickup)),
        (vendor.clone(), Action::RegradeLine, deal(DealStatus::Weighed)),
        (seller.clone(), Action::ConfirmWeights, deal(DealStatus::Weighed)),
        (vendor.clone(), Action::RequestPayment, deal(DealStatus::SellerConfirmed)),
        (vendor.clone(), Action::MarkPaidDirect, deal(DealStatus::SellerConfirmed)),
        (seller.clone(), Action::ConfirmReceived, deal(DealStatus::SellerConfirmed)),
        (transporter.clone(), Action::Handover, deal(DealStatus::Paid)),
        (vendor.clone(), Action::ConfirmDelivery, deal(DealStatus::InTransit)),
        (seller.clone(), Action::CancelDeal, deal(DealStatus::Created)),
        (seller.clone(), Action::OpenDispute, deal(DealStatus::Weighed)),
        (vendor.clone(), Action::RateDeal, deal(DealStatus::Delivered)),
        (transporter.clone(), Action::StartCall, deal(DealStatus::JobAssigned)),
        (vendor.clone(), Action::AssignTransporter, deal(DealStatus::Created)),
        (seller.clone(), Action::ViewReceipt, deal(DealStatus::Closed)),
        (transporter.clone(), Action::ViewJob, job(JobStatus::Offered, false)),
        (transporter.clone(), Action::AcceptJob, job(JobStatus::Offered, false)),
        (transporter.clone(), Action::DeclineJob, job(JobStatus::Offered, false)),
        (transporter.clone(), Action::UpdateJob, job(JobStatus::EnRoute, true)),
        (transporter.clone(), Action::PostJobPosition, job(JobStatus::EnRoute, true)),
        (seller.clone(), Action::ViewThread, thread.clone()),
        (vendor.clone(), Action::PostMessage, thread.clone()),
        (admin.clone(), Action::ManageDrive, drive.clone()),
        (seller.clone(), Action::JoinDrive, drive.clone()),
        (admin.clone(), Action::ViewDriveReport, drive.clone()),
        (ops.clone(), Action::OpsReviewKyc, Resource::Platform),
        (ops.clone(), Action::OpsReviewAi, published.clone()),
        (ops.clone(), Action::OpsResolveDispute, deal(DealStatus::Disputed)),
        (ops.clone(), Action::OpsManageTaxonomy, Resource::Platform),
        (ops.clone(), Action::OpsManageConfig, Resource::Platform),
        (ops.clone(), Action::OpsViewFraud, Resource::Platform),
        (ops.clone(), Action::OpsPublishRateIndex, Resource::Platform),
    ];
    for a in Action::ALL {
        assert!(cases.iter().any(|(_, ca, _)| ca == a), "{} has no allowing case", a.as_str());
    }
    for (p, a, r) in &cases {
        assert!(w.allowed(p, *a, r), "{} should be allowed for {:?}", a.as_str(), p);
    }
}

#[test]
fn entities_for_is_accepted_by_the_engine_for_every_resource_kind() {
    let w = World::new();
    let schema = kabados_authz::schema().unwrap();
    let mut full = w.listing(ListingStatus::Reserved);
    full.accepted_vendor = Some(w.vendor.clone());
    full.deal_status = Some(DealStatus::Paid);
    full.assigned_transporter = Some(w.transporter.clone());
    full.job_status = Some(JobStatus::Accepted);
    let resources = vec![
        Resource::Listing(w.listing(ListingStatus::Draft)),
        Resource::Listing(full.clone()),
        Resource::Deal(w.deal(DealStatus::Created)),
        Resource::Offer(OfferAttrs { id: OfferId::new(), listing: full, vendor: w.vendor.clone(), status: OfferStatus::Accepted }),
        Resource::Job(JobAttrs {
            id: JobId::new(),
            deal: w.deal(DealStatus::JobAssigned),
            transporter: None,
            offered_to: vec![],
            status: JobStatus::Offered,
        }),
        Resource::Thread(ThreadAttrs {
            id: ThreadId::new(),
            seller: w.seller.clone(),
            vendor: w.vendor.clone(),
            listing_status: ListingStatus::Published,
            deal_open: true,
        }),
        Resource::Drive(DriveAttrs { id: DriveId::new(), admin: w.admin.clone(), status: DriveStatus::Completed }),
        Resource::Platform,
    ];
    for r in &resources {
        for (who, p) in w.everyone() {
            let json = entities_for(&p, r);
            cedar_policy::Entities::from_json_value(json, Some(&schema)).unwrap_or_else(|e| panic!("{who} {r:?}: {e}"));
        }
    }
}

#[tokio::test]
async fn batch_matches_single_decisions() {
    let w = World::new();
    let reqs: Vec<(Principal, Action, Resource)> =
        LISTING.iter().map(|&ls| (w.vendor_p(&w.vendor), Action::MakeOffer, Resource::Listing(w.listing(ls)))).collect();
    let batch = w.engine.batch_is_authorized(&reqs).await.unwrap();
    for ((p, a, r), d) in reqs.iter().zip(batch) {
        assert_eq!(w.engine.is_authorized(p, *a, r).await.unwrap(), d);
    }
}
