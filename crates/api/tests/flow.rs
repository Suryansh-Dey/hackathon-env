//! End-to-end API flows through the real router on the in-memory backend.

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use kabados_api::AppState;
use kabados_api::tasks::RecordingQueue;
use kabados_db::memory::MemoryStore;
use kabados_db::repo::Db;
use kabados_integrations::Integrations;
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

struct Api {
    app: Router,
    db: Db,
}

impl Api {
    fn new() -> Self {
        let mut cfg = kabados_runtime::Config::from_env();
        cfg.env = "local".into();
        cfg.first_look_minutes = 0;
        let db = Db::new(Arc::new(MemoryStore::new()));
        let root = std::env::temp_dir().join(format!("kabados-test-{}", kabados_domain::ids::EventId::new()));
        let ix = Integrations::local(db.clone(), "local", root, "http://localhost:8080");
        let st = AppState::local(cfg, db.clone(), ix, Arc::new(RecordingQueue::default())).unwrap();
        Self { app: kabados_api::app(st), db }
    }

    async fn call(&self, method: Method, path: &str, token: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
        self.call_with_key(method, path, token, body, None).await
    }

    async fn call_with_key(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
        key: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method.clone()).uri(path);
        if body.is_some() {
            req = req.header("content-type", "application/json");
        }
        if let Some(t) = token {
            req = req.header("authorization", format!("Bearer {t}"));
        }
        if method == Method::POST {
            let k = key.map(str::to_owned).unwrap_or_else(|| kabados_domain::ids::EventId::new().to_string());
            req = req.header("idempotency-key", k);
        }
        let req = req.body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty)).unwrap();
        let res = self.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), 10 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    async fn ok(&self, method: Method, path: &str, token: &str, body: Option<Value>) -> Value {
        let (s, v) = self.call(method.clone(), path, Some(token), body).await;
        assert!(s.is_success(), "{method} {path} → {s}: {v}");
        v
    }

    async fn token(&self, phone: &str) -> String {
        let (s, v) = self.call(Method::POST, "/dev/token", None, Some(json!({"phone": phone}))).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v["token"].as_str().unwrap().to_owned()
    }
}

async fn seller_with_listing(api: &Api, phone: &str) -> (String, String) {
    let t = api.token(phone).await;
    api.ok(Method::GET, "/v1/me", &t, None).await;
    api.ok(Method::PATCH, "/v1/me", &t, Some(json!({"name": "Asha", "adult_confirmed": true}))).await;
    let addr = api
        .ok(Method::POST, "/v1/me/addresses", &t, Some(json!({"label": "Home", "location": {"lat": 28.6300, "lng": 77.2200}, "text": "Flat 4B, Barakhamba Road, New Delhi", "landmark": "Near metro gate 2", "city": "delhi"})))
        .await;
    let l = api
        .ok(Method::POST, "/v1/listings", &t, Some(json!({"address_id": addr["id"], "description": "old copper wire and newspapers"})))
        .await;
    let id = l["listing"]["id"].as_str().unwrap().to_owned();
    api.ok(
        Method::PATCH,
        &format!("/v1/listings/{id}/items"),
        &t,
        Some(json!({"items": [
            {"category_id": "metal.copper", "grade": "clean", "quantity": 1, "est_weight_g": 10000},
            {"category_id": "paper.newspaper", "quantity": 1, "est_weight_g": 60000}
        ]})),
    )
    .await;
    (t, id)
}

#[tokio::test]
async fn direct_payment_deal_end_to_end() {
    let api = Api::new();
    let (s, seeded) = api.call(Method::POST, "/dev/seed", None, None).await;
    assert_eq!(s, StatusCode::OK, "{seeded}");

    let (seller, lid) = seller_with_listing(&api, "+919876543210").await;
    let sug = api.ok(Method::GET, &format!("/v1/listings/{lid}/price-suggestion"), &seller, None).await;
    assert!(sug["vendor_count"].as_u64().unwrap() >= 1, "{sug}");

    let pubd = api.ok(Method::POST, &format!("/v1/listings/{lid}:publish"), &seller, None).await;
    assert_eq!(pubd["listing"]["status"], "PUBLISHED");

    // A vendor (seeded) sees the listing without the exact address.
    let vendor = api.token("+919800000003").await;
    let lv = api.ok(Method::GET, &format!("/v1/listings/{lid}"), &vendor, None).await;
    assert_eq!(lv["viewer"], "VENDOR");
    assert!(lv["exact_address"].is_null(), "exact address must not leak before a deal");

    let q = api.ok(Method::GET, &format!("/v1/listings/{lid}/my-quote"), &vendor, None).await;
    assert_eq!(q["eligible"], true, "{q}");

    let offer = api
        .ok(Method::POST, &format!("/v1/listings/{lid}/offers"), &vendor, Some(json!({"pickup_method": "SELF_PICKUP", "payment_mode": "B_DIRECT", "slot": {"date": "2030-01-05", "window": "09:00-13:00"}})))
        .await;
    let oid = offer["id"].as_str().unwrap().to_owned();

    // A stranger cannot accept the seller's offer.
    let stranger = api.token("+919811111111").await;
    let (s, _) = api.call(Method::POST, &format!("/v1/offers/{oid}:accept"), Some(&stranger), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let offers = api.ok(Method::GET, &format!("/v1/listings/{lid}/offers"), &seller, None).await;
    assert_eq!(offers.as_array().unwrap().len(), 1);

    // Idempotent replay returns the same deal.
    let key = "accept-key-0001";
    let (s1, a1) = api.call_with_key(Method::POST, &format!("/v1/offers/{oid}:accept"), Some(&seller), None, Some(key)).await;
    let (s2, a2) = api.call_with_key(Method::POST, &format!("/v1/offers/{oid}:accept"), Some(&seller), None, Some(key)).await;
    assert_eq!((s1, s2), (StatusCode::OK, StatusCode::OK), "{a1} {a2}");
    assert_eq!(a1, a2);
    let did = a1["deal_id"].as_str().unwrap().to_owned();

    let dv = api.ok(Method::GET, &format!("/v1/deals/{did}"), &vendor, None).await;
    assert_eq!(dv["deal"]["status"], "CREATED");
    assert!(dv["exact_address"]["text"].as_str().unwrap().contains("Barakhamba"), "vendor of an active deal sees the address");
    let (s, _) = api.call(Method::GET, &format!("/v1/deals/{did}"), Some(&stranger), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    api.ok(Method::POST, &format!("/v1/deals/{did}:arrive"), &vendor, None).await;
    api.ok(Method::POST, &format!("/v1/deals/{did}/weighings"), &vendor, Some(json!({"line_n": 1, "reading_g": 10200}))).await;
    let w = api.ok(Method::POST, &format!("/v1/deals/{did}/weighings"), &vendor, Some(json!({"line_n": 2, "reading_g": 58000}))).await;
    assert_eq!(w["deal"]["status"], "WEIGHED");
    let final_amount = w["deal"]["final_amount"].as_i64().unwrap();
    assert!(final_amount > 0);

    let (s, _) = api.call(Method::POST, &format!("/v1/deals/{did}:confirm-weights"), Some(&vendor), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "only the seller confirms weights");
    api.ok(Method::POST, &format!("/v1/deals/{did}:confirm-weights"), &seller, None).await;
    api.ok(Method::POST, &format!("/v1/deals/{did}:mark-paid-direct"), &vendor, None).await;
    let paid = api.ok(Method::POST, &format!("/v1/deals/{did}:confirm-received"), &seller, None).await;
    assert_eq!(paid["deal"]["status"], "PAID_DIRECT");
    let otp = paid["handover_otp"].as_str().expect("seller sees the handover code").to_owned();
    let vv = api.ok(Method::GET, &format!("/v1/deals/{did}"), &vendor, None).await;
    assert!(vv["handover_otp"].is_null(), "vendor never sees the code");

    let (s, _) = api.call(Method::POST, &format!("/v1/deals/{did}:handover"), Some(&vendor), Some(json!({"otp": "0000x"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    api.ok(Method::POST, &format!("/v1/deals/{did}:handover"), &vendor, Some(json!({"otp": otp}))).await;
    let done = api.ok(Method::POST, &format!("/v1/deals/{did}:confirm-delivery"), &vendor, None).await;
    assert_eq!(done["deal"]["status"], "SETTLED");

    let r = api.ok(Method::GET, &format!("/v1/deals/{did}/receipt"), &seller, None).await;
    assert_eq!(r["receipt"]["total_weight"], 68_200);
    let path = r["verify_path"].as_str().unwrap();
    let (rid, sig) = path.trim_start_matches("/receipt?id=").split_once("&sig=").unwrap();
    let (s, pubr) = api.call(Method::GET, &format!("/v1/public/receipts/{rid}?sig={sig}"), None, None).await;
    assert_eq!(s, StatusCode::OK, "{pubr}");
    let (s, _) = api.call(Method::GET, &format!("/v1/public/receipts/{rid}?sig=deadbeefdeadbeef"), None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    api.ok(Method::POST, &format!("/v1/deals/{did}/ratings"), &seller, Some(json!({"stars": 5}))).await;
    let (s, _) = api.call(Method::POST, &format!("/v1/deals/{did}/ratings"), Some(&seller), Some(json!({"stars": 4}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "one rating per party per deal");

    // Mode B platform fee was posted to the ledger.
    let vendor_id = seeded["vendors"][2]["user_id"].as_str().unwrap();
    assert!(api.db.balance(&format!("vendor_payable:{vendor_id}")).await.unwrap().0 < 0);
}

#[tokio::test]
async fn platform_payment_goes_through_signed_webhook() {
    let api = Api::new();
    api.call(Method::POST, "/dev/seed", None, None).await;
    let (seller, lid) = seller_with_listing(&api, "+919876500000").await;
    api.ok(Method::POST, &format!("/v1/listings/{lid}:publish"), &seller, None).await;
    let vendor = api.token("+919800000001").await;
    let offer = api
        .ok(
            Method::POST,
            &format!("/v1/listings/{lid}/offers"),
            &vendor,
            Some(json!({"pickup_method": "SELF_PICKUP", "payment_mode": "A_PLATFORM"})),
        )
        .await;
    let a = api.ok(Method::POST, &format!("/v1/offers/{}:accept", offer["id"].as_str().unwrap()), &seller, None).await;
    let did = a["deal_id"].as_str().unwrap().to_owned();
    api.ok(Method::POST, &format!("/v1/deals/{did}:arrive"), &vendor, None).await;
    api.ok(Method::POST, &format!("/v1/deals/{did}/weighings"), &vendor, Some(json!({"line_n": 1, "reading_g": 9800}))).await;
    api.ok(Method::POST, &format!("/v1/deals/{did}/weighings"), &vendor, Some(json!({"line_n": 2, "reading_g": 61000}))).await;
    api.ok(Method::POST, &format!("/v1/deals/{did}:confirm-weights"), &seller, None).await;
    let pr = api.ok(Method::POST, &format!("/v1/deals/{did}/payment-request"), &vendor, None).await;
    let order = pr["order"]["id"].as_str().unwrap().to_owned();
    assert!(pr["order"]["simulated"].as_bool().unwrap());

    // Forged webhook is rejected.
    let (s, _) = api.call(Method::POST, "/webhooks/razorpay", None, Some(json!({"event": "payment.captured"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    api.ok(Method::POST, &format!("/v1/payments/{order}:simulate-capture"), &vendor, None).await;
    let paid = api.ok(Method::GET, &format!("/v1/deals/{did}"), &seller, None).await;
    assert_eq!(paid["deal"]["status"], "PAID");
    assert!(paid["handover_otp"].as_str().is_some());
    let seller_id = paid["deal"]["seller_id"].as_str().unwrap();
    let final_amount = paid["deal"]["final_amount"].as_i64().unwrap();
    assert_eq!(api.db.balance(&format!("seller_receivable:{seller_id}")).await.unwrap().0, final_amount);
    // A second capture attempt on an already-paid deal is a no-op (no double-post).
    let (s, _) = api.call(Method::POST, &format!("/v1/payments/{order}:simulate-capture"), Some(&vendor), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(api.db.balance(&format!("seller_receivable:{seller_id}")).await.unwrap().0, final_amount, "capture must not double-post");
}

#[tokio::test]
async fn post_without_idempotency_key_is_rejected_and_openapi_is_served() {
    let api = Api::new();
    let t = api.token("+919876511111").await;
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/listings")
        .header("authorization", format!("Bearer {t}"))
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let res = api.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let (s, spec) = api.call(Method::GET, "/v1/public/openapi.json", None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(spec["paths"].as_object().unwrap().len() > 60);
    let (s, _) = api.call(Method::GET, "/v1/me", None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}
