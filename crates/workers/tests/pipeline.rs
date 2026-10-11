//! Offline worker-pipeline tests: the analysis worker on the mock AI model and
//! the in-memory store (no network, no AWS).

use kabados_api::AppState;
use kabados_api::tasks::{RecordingQueue, Task};
use kabados_db::memory::MemoryStore;
use kabados_db::repo::Db;
use kabados_domain::geo::LatLng;
use kabados_domain::ids::*;
use kabados_domain::listing::*;
use kabados_domain::state::StateMachine;
use kabados_integrations::Integrations;
use kabados_workers::{WorkerCtx, analysis};
use std::sync::Arc;

fn ctx() -> (WorkerCtx, Db) {
    // SAFETY: tests are single-threaded here; the mock AI model needs AI_MODE=mock.
    unsafe {
        std::env::set_var("AI_MODE", "mock");
        std::env::set_var("KABADOS_ENV", "local");
    }
    let mut cfg = kabados_runtime::Config::from_env();
    cfg.env = "local".into();
    let db = Db::new(Arc::new(MemoryStore::new()));
    let root = std::env::temp_dir().join(format!("kabados-wk-{}", EventId::new()));
    let ix = Integrations::local(db.clone(), "local", root, "http://localhost:8080");
    let app = AppState::local(cfg, db.clone(), ix, Arc::new(RecordingQueue::default())).unwrap();
    (WorkerCtx::new(app), db)
}

async fn draft_listing(db: &Db) -> ListingId {
    let seller = UserId::new();
    let p = LatLng { lat: 28.6139, lng: 77.209 };
    let id = ListingId::new();
    let listing = Listing {
        id: id.clone(),
        seller_id: seller.clone(),
        status: ListingStatus::Analyzing,
        current_version: 1,
        published_version: None,
        address_id: AddressId::new(),
        location: p,
        approx: ApproxLocation { h3_r7: p.cell_r7().unwrap(), h3_r8: p.cell_r8().unwrap(), center: p, city: "delhi".into() },
        drive_id: None,
        title: "New listing".into(),
        contains_hazardous: false,
        categories: vec![],
        total_est_weight_g: kabados_domain::Grams(0),
        min_ladder: kabados_domain::ladder::LadderLevel::L0Claimed,
        published_at: None,
        first_look_until: None,
        expires_at: None,
        accepted_offer_id: None,
        deal_id: None,
        ai_flagged_for_review: false,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let version = ListingVersion {
        listing_id: id.clone(),
        ver: 1,
        description: Some("two old ceiling fans, a lot of copper wire, and a stack of newspapers".into()),
        asking_price: None,
        claim_reason: None,
        pickup: PickupPreferences::default(),
        analysis: AnalysisState {
            status: AnalysisStatus::Queued,
            proof_rounds: 0,
            model: None,
            prompt_version: None,
            updated_at: chrono::Utc::now(),
        },
        created_at: chrono::Utc::now(),
    };
    db.create_listing(listing, version).await.unwrap();
    id
}

#[tokio::test]
async fn analyze_listing_itemizes_and_advances_status() {
    let (ctx, db) = ctx();
    let id = draft_listing(&db).await;

    analysis::handle_task(&ctx, Task::AnalyzeListing { listing_id: id.clone(), ver: 1 }).await.unwrap();

    let bundle = db.listing_bundle(&id, None).await.unwrap().unwrap();
    assert!(!bundle.items.is_empty(), "the mock itemizer should produce items from the description");
    // Every item maps to a real taxonomy category and AI-sourced items carry a confidence.
    let tax = kabados_domain::taxonomy::Taxonomy::seed();
    for i in &bundle.items {
        assert!(tax.get(&i.data.category_id).is_some(), "unknown category {}", i.data.category_id);
        assert_eq!(i.data.source, ItemSource::Ai);
    }
    // Analysis left the listing in a terminal-for-analysis state the seller can act on.
    let status = bundle.listing.data.status;
    assert!(matches!(status, ListingStatus::Ready | ListingStatus::NeedsInput), "status was {status:?}");
    assert_eq!(bundle.version.data.analysis.status, AnalysisStatus::Done);
    // Hazardous recall: ceiling fans are e-waste → the listing must be flagged hazardous.
    assert!(bundle.listing.data.contains_hazardous, "ceiling fan e-waste should mark the lot hazardous");
    assert!(ListingStatus::Analyzing.can_transition_to(status));
}
