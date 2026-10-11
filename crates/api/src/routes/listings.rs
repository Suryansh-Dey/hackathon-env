//! Seller listings (PLAN.md §6.1, §17.2 "Seller listings").

use super::{id_action, now, parse, split_action};
use crate::access::*;
use crate::auth::Me;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::tasks::Task;
use crate::views::*;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use chrono::Duration;
use kabados_authz::{Action, Resource};
use kabados_db::repo::ListingBundle;
use kabados_db::{Record, TxOp};
use kabados_domain::events::DomainEvent;
use kabados_domain::geo;
use kabados_domain::ids::*;
use kabados_domain::ladder::LadderLevel;
use kabados_domain::listing::*;
use kabados_domain::state::StateMachine;
use kabados_domain::taxonomy::Taxonomy;
use kabados_domain::user::Role;
use kabados_domain::{Grams, Paise};
use kabados_integrations::blob::PresignedUpload;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use utoipa::ToSchema;

pub const LISTING_TTL_DAYS: i64 = 14;
pub const MAX_IMAGES: usize = 20;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/listings", get(my_listings).post(create_listing))
        .route("/v1/listings/{id}", get(get_listing).patch(edit_listing).post(listing_action))
        .route("/v1/listings/{id}/items", patch(replace_items))
        .route("/v1/listings/{id}/images/{seg}", post(image_action))
        .route("/v1/listings/{id}/claims/{seg}", post(claim_action))
        .route("/v1/listings/{id}/proofs", post(provide_proof))
        .route("/v1/listings/{id}/price-suggestion", get(price_suggestion))
        .route("/v1/listings/{id}/images:presign", post(presign_image_route))
        .route("/v1/listings/{id}/proofs:skip", post(skip_proofs_route))
}

pub async fn load_bundle(st: &AppState, id: &ListingId) -> ApiResult<ListingBundle> {
    st.db.listing_bundle(id, None).await?.ok_or(ApiError::NotFound("listing"))
}

async fn as_seller(st: &AppState, me: &Me, b: &ListingBundle, a: Action) -> ApiResult<()> {
    let (deal, job, drive) = listing_context(st, &b.listing.data).await?;
    ensure(st, &seller(me.id()), a, &Resource::Listing(listing_attrs(&b.listing.data, deal.as_ref(), job.as_ref(), drive.as_ref()))).await
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewListing {
    pub address_id: AddressId,
    pub description: Option<String>,
    pub drive_id: Option<DriveId>,
    #[serde(default)]
    pub pickup: PickupPreferences,
}

#[utoipa::path(post, path = "/v1/listings", tag = "listings", request_body = NewListing, responses((status = 201, body = ListingView)))]
pub async fn create_listing(State(st): State<AppState>, me: Me, Json(b): Json<NewListing>) -> ApiResult<(StatusCode, Json<ListingView>)> {
    if !me.user.data.adult_confirmed {
        return Err(ApiError::bad("confirm you are 18 or older first"));
    }
    let addr = st.db.address(me.id(), &b.address_id).await?.ok_or(ApiError::NotFound("address"))?.data;
    validate_text(&b.description, 2000, "description")?;
    for s in &b.pickup.slots {
        if kabados_domain::ruleset::parse_window(s).is_none() {
            return Err(ApiError::bad(format!("{s:?} is not a HH:MM-HH:MM window")));
        }
    }
    if let Some(d) = &b.drive_id {
        let drive = st.db.drive(d).await?.ok_or(ApiError::NotFound("drive"))?;
        if drive.data.status != kabados_domain::drive::DriveStatus::Open {
            return Err(ApiError::conflict("this drive is closed"));
        }
    }
    let id = ListingId::new();
    let center = geo::cell_center(&addr.h3_r7)?;
    let listing = Listing {
        id: id.clone(),
        seller_id: me.id().clone(),
        status: ListingStatus::Draft,
        current_version: 1,
        published_version: None,
        address_id: addr.id.clone(),
        location: addr.location,
        approx: ApproxLocation { h3_r7: addr.h3_r7.clone(), h3_r8: addr.h3_r8.clone(), center, city: addr.city.clone() },
        drive_id: b.drive_id,
        title: "New listing".into(),
        contains_hazardous: false,
        categories: vec![],
        total_est_weight_g: Grams(0),
        min_ladder: LadderLevel::L0Claimed,
        published_at: None,
        first_look_until: None,
        expires_at: None,
        accepted_offer_id: None,
        deal_id: None,
        ai_flagged_for_review: false,
        created_at: now(),
        updated_at: now(),
    };
    let version = ListingVersion {
        listing_id: id.clone(),
        ver: 1,
        description: b.description.map(|d| d.trim().to_owned()).filter(|d| !d.is_empty()),
        asking_price: None,
        claim_reason: None,
        pickup: b.pickup,
        analysis: AnalysisState {
            status: AnalysisStatus::NotStarted,
            proof_rounds: 0,
            model: None,
            prompt_version: None,
            updated_at: now(),
        },
        created_at: now(),
    };
    st.db.create_listing(listing, version).await?;
    let bundle = load_bundle(&st, &id).await?;
    Ok((StatusCode::CREATED, Json(build_view(&st, &bundle, Viewer::Seller, None).await?)))
}

fn validate_text(t: &Option<String>, max: usize, what: &str) -> ApiResult<()> {
    if t.as_ref().is_some_and(|s| s.chars().count() > max) {
        return Err(ApiError::bad(format!("{what} is too long (max {max} characters)")));
    }
    Ok(())
}

#[utoipa::path(get, path = "/v1/listings", tag = "listings", responses((status = 200, body = Vec<ListingSummary>)))]
pub async fn my_listings(State(st): State<AppState>, me: Me) -> ApiResult<Json<Vec<ListingSummary>>> {
    let (recs, _) = st.db.seller_listings(me.id(), None, 200).await?;
    Ok(Json(recs.iter().map(|r| ListingSummary::from(&r.data)).collect()))
}

async fn image_views(st: &AppState, imgs: &[Record<ImageMeta>], viewer: Viewer) -> ApiResult<Vec<ImageView>> {
    let ttl = std::time::Duration::from_secs(3600);
    let mut out = Vec::new();
    for i in imgs.iter().map(|r| &r.data).filter(|i| i.uploaded) {
        // Vendors and transporters only ever see processed copies (faces blurred, EXIF stripped).
        let key = match viewer {
            Viewer::Seller | Viewer::Ops => i.processed_key.as_ref().or(Some(&i.original_key)),
            _ => i.processed_key.as_ref(),
        };
        let url = match key {
            Some(k) => Some(st.ix.blobs.presign_get(&st.cfg.media_bucket, k, ttl).await?),
            None => None,
        };
        let thumb_url = match &i.thumb_key {
            Some(k) => Some(st.ix.blobs.presign_get(&st.cfg.media_bucket, k, ttl).await?),
            None => None,
        };
        out.push(ImageView {
            id: i.id.clone(),
            kind: i.kind,
            source: i.source,
            url,
            thumb_url,
            triage: i.triage.clone(),
            proof_request_id: i.proof_request_id.clone(),
            processed: i.processed_key.is_some(),
        });
    }
    Ok(out)
}

pub async fn exact_address(st: &AppState, l: &Listing) -> ApiResult<Option<ExactAddress>> {
    let Some(a) = st.db.address(&l.seller_id, &l.address_id).await? else { return Ok(None) };
    let landmark = match &a.data.landmark {
        Some(x) => Some(st.sealer.open(x).await?),
        None => None,
    };
    Ok(Some(ExactAddress { location: a.data.location, text: st.sealer.open(&a.data.text).await?, landmark }))
}

pub async fn build_view(st: &AppState, b: &ListingBundle, viewer: Viewer, exact: Option<ExactAddress>) -> ApiResult<ListingView> {
    let l = &b.listing.data;
    let honesty = st.db.user(&l.seller_id).await?.map(|u| u.data.honesty.smoothed()).unwrap_or(0.8);
    let items: Vec<Item> = b.item_data();
    let weight_notes = if viewer == Viewer::Seller {
        items.iter().filter(|i| i.estimate_outside_plausible()).map(|i| i.id.clone()).collect()
    } else {
        vec![]
    };
    Ok(ListingView {
        viewer,
        listing: ListingSummary::from(l),
        version: ListingVersionView::from(&b.version.data),
        items,
        claims: b.claims.iter().map(|r| r.data.clone()).collect(),
        proof_requests: b.proofs.iter().map(|r| r.data.clone()).collect(),
        images: image_views(st, &b.images, viewer).await?,
        exact_address: exact,
        deal_id: l.deal_id.clone(),
        seller_honesty: honesty,
        weight_notes,
    })
}

/// Listing detail; what is shown depends on who is asking (PLAN.md §7.3, §18).
#[utoipa::path(get, path = "/v1/listings/{id}", tag = "listings", responses((status = 200, body = ListingView)))]
pub async fn get_listing(State(st): State<AppState>, me: Me, Path(seg): Path<String>) -> ApiResult<Json<ListingView>> {
    let id: ListingId = parse(&seg, "listing id")?;
    let b = load_bundle(&st, &id).await?;
    let l = &b.listing.data;
    let (deal, job, drive) = listing_context(&st, l).await?;
    let res = Resource::Listing(listing_attrs(l, deal.as_ref(), job.as_ref(), drive.as_ref()));
    let mut candidates = vec![(seller(me.id()), Viewer::Seller)];
    if let Some(v) = st.db.vendor(me.id()).await? {
        candidates.push((vendor_principal(&v.data), Viewer::Vendor));
    }
    if let Some(t) = st.db.transporter(me.id()).await? {
        candidates.push((transporter_principal(&t.data), Viewer::Transporter));
    }
    for (p, viewer) in candidates {
        if allowed(&st, &p, Action::ViewListing, &res).await? {
            let exact = if allowed(&st, &p, Action::ViewExactAddress, &res).await? { exact_address(&st, l).await? } else { None };
            return Ok(Json(build_view(&st, &b, viewer, exact).await?));
        }
    }
    Err(ApiError::NotFound("listing"))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct EditListing {
    pub description: Option<String>,
    /// Asking price for the whole lot, with a reason (PLAN.md §6.1 step 5).
    pub asking_price: Option<Paise>,
    pub claim_reason: Option<String>,
    pub pickup: Option<PickupPreferences>,
    /// Clears the asking price (accept the suggestion instead).
    #[serde(default)]
    pub clear_asking_price: bool,
}

/// Edits; a published listing gets a new version (quotes and offers are per version).
#[utoipa::path(patch, path = "/v1/listings/{id}", tag = "listings", request_body = EditListing, responses((status = 200, body = ListingView)))]
pub async fn edit_listing(
    State(st): State<AppState>,
    me: Me,
    Path(seg): Path<String>,
    Json(b): Json<EditListing>,
) -> ApiResult<Json<ListingView>> {
    let id: ListingId = parse(&seg, "listing id")?;
    let mut bundle = load_bundle(&st, &id).await?;
    as_seller(&st, &me, &bundle, Action::EditListing).await?;
    if !bundle.listing.data.status.is_editable() {
        return Err(ApiError::conflict("this listing can't be edited now"));
    }
    validate_text(&b.description, 2000, "description")?;
    validate_text(&b.claim_reason, 500, "claim reason")?;
    if b.asking_price.is_some_and(|p| p.0 <= 0) {
        return Err(ApiError::bad("asking price must be positive"));
    }
    let mut v = bundle.version.data.clone();
    if let Some(d) = b.description {
        v.description = Some(d.trim().to_owned()).filter(|d| !d.is_empty());
    }
    if b.clear_asking_price {
        v.asking_price = None;
        v.claim_reason = None;
    }
    if let Some(p) = b.asking_price {
        v.asking_price = Some(p);
    }
    if let Some(r) = b.claim_reason {
        v.claim_reason = Some(r.trim().to_owned()).filter(|r| !r.is_empty());
    }
    if let Some(p) = b.pickup {
        v.pickup = p;
    }
    if bundle.listing.data.status == ListingStatus::Published {
        new_version(&st, &mut bundle, v).await?;
    } else {
        let rec = Record::new(bundle.version.meta.clone(), v);
        st.db.write_ops(vec![st.db.version_op(&rec)?]).await?;
    }
    let bundle = load_bundle(&st, &id).await?;
    Ok(Json(build_view(&st, &bundle, Viewer::Seller, None).await?))
}

/// Copies the current version's children into version `n+1` and bumps the listing.
async fn new_version(st: &AppState, b: &mut ListingBundle, mut v: ListingVersion) -> ApiResult<()> {
    let id = b.listing.data.id.clone();
    let next = b.listing.data.current_version + 1;
    v.ver = next;
    v.created_at = now();
    let mut ops = vec![st.db.version_op(&Record::new(
        kabados_db::Meta::new(kabados_db::keys::listing(&id), kabados_db::keys::listing_version(next), "listing_version"),
        v,
    ))?];
    for i in &b.items {
        ops.push(st.db.item_op(&id, next, &i.data)?);
    }
    for c in &b.claims {
        ops.push(st.db.claim_op(&id, next, &c.data)?);
    }
    for p in &b.proofs {
        ops.push(st.db.proof_op(&id, next, &p.data)?);
    }
    for img in &b.images {
        ops.push(st.db.image_op(&id, next, &img.data)?);
    }
    st.db.write_ops(ops).await?;
    b.listing.data.current_version = next;
    b.listing.data.published_version = Some(next);
    b.listing.data.updated_at = now();
    st.db.save_listing_with(&mut b.listing, vec![], vec![DomainEvent::ListingVersionCreated { listing_id: id, ver: next }], vec![]).await?;
    Ok(())
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ItemInput {
    /// Existing item to keep (AI fields are preserved when the category is unchanged).
    pub id: Option<ItemId>,
    pub category_id: String,
    pub grade: Option<String>,
    #[serde(default)]
    pub attributes: BTreeMap<String, AttrValue>,
    pub quantity: u32,
    /// Seller's estimate for the whole line (required before publishing).
    pub est_weight_g: Option<Grams>,
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReplaceItems {
    pub items: Vec<ItemInput>,
}

/// Seller edits, adds, splits or merges items (PLAN.md §6.1 step 3–4). The
/// system never overwrites the seller's weight; hazard flags come from the taxonomy.
#[utoipa::path(patch, path = "/v1/listings/{id}/items", tag = "listings", request_body = ReplaceItems, responses((status = 200, body = ListingView)))]
pub async fn replace_items(
    State(st): State<AppState>,
    me: Me,
    Path(id): Path<String>,
    Json(body): Json<ReplaceItems>,
) -> ApiResult<Json<ListingView>> {
    let id: ListingId = parse(&id, "listing id")?;
    let mut b = load_bundle(&st, &id).await?;
    as_seller(&st, &me, &b, Action::EditListing).await?;
    if !b.listing.data.status.is_editable() || b.listing.data.status == ListingStatus::Analyzing {
        return Err(ApiError::conflict("items can't be edited right now"));
    }
    if body.items.len() > 50 {
        return Err(ApiError::bad("at most 50 items per listing"));
    }
    let tax = st.taxonomy().await?;
    let existing: BTreeMap<ItemId, Item> = b.items.iter().map(|r| (r.data.id.clone(), r.data.clone())).collect();
    let mut items = Vec::with_capacity(body.items.len());
    for input in body.items {
        items.push(item_from_input(&tax, input, &existing)?);
    }
    let ver = b.listing.data.current_version;
    if b.listing.data.status == ListingStatus::Published {
        let v = b.version.data.clone();
        new_version(&st, &mut b, v).await?;
        b = load_bundle(&st, &id).await?;
    }
    let ver = b.listing.data.current_version.max(ver);
    // Kept items are overwritten in place: a transaction can't delete and put the same key.
    let kept: BTreeSet<&ItemId> = items.iter().map(|i| &i.id).collect();
    if kept.len() != items.len() {
        return Err(ApiError::bad("each item id may appear only once"));
    }
    let mut ops: Vec<TxOp> =
        b.items.iter().filter(|r| !kept.contains(&r.data.id)).map(|r| st.db.delete_item_op(&id, ver, &r.data.id)).collect();
    for i in &items {
        ops.push(st.db.item_op(&id, ver, i)?);
    }
    st.db.write_ops(ops).await?;
    let (total, cats, hazardous, min_ladder) = summarize(&items);
    b.listing.data.total_est_weight_g = total;
    b.listing.data.categories = cats;
    b.listing.data.contains_hazardous = hazardous;
    b.listing.data.min_ladder = min_ladder;
    b.listing.data.title = title_for(&tax, &items);
    b.listing.data.updated_at = now();
    st.db.save_listing(&mut b.listing).await?;
    let b = load_bundle(&st, &id).await?;
    Ok(Json(build_view(&st, &b, Viewer::Seller, None).await?))
}

pub fn item_from_input(tax: &Taxonomy, i: ItemInput, existing: &BTreeMap<ItemId, Item>) -> ApiResult<Item> {
    let cat = tax.require(&i.category_id)?;
    if let Some(g) = &i.grade
        && !cat.has_grade(g)
    {
        return Err(ApiError::bad(format!("{g:?} is not a grade of {}", cat.name)));
    }
    if i.quantity == 0 || i.quantity > 10_000 {
        return Err(ApiError::bad("quantity must be between 1 and 10,000"));
    }
    if i.est_weight_g.is_some_and(|w| w.0 <= 0 || w.0 > 100_000_000) {
        return Err(ApiError::bad("weight must be between 1 g and 100 tonnes"));
    }
    let prev = i.id.as_ref().and_then(|id| existing.get(id));
    let same_cat = prev.is_some_and(|p| p.category_id == i.category_id);
    let plausible = cat
        .reference_weight_g
        .map(|r| PlausibleWeight { range: r, source: WeightSource::ReferenceTable })
        .or_else(|| prev.filter(|_| same_cat).and_then(|p| p.plausible_weight));
    Ok(Item {
        id: i.id.unwrap_or_else(ItemId::new),
        category_id: cat.id.clone(),
        grade: i.grade,
        attributes: i.attributes,
        quantity: i.quantity,
        unit: cat.unit,
        est_weight_g: i.est_weight_g,
        plausible_weight: plausible,
        hazard: Hazard {
            // Hazard recall first: the taxonomy flag or a previous AI flag on the same category stays.
            is_hazardous: cat.hazardous || (same_cat && prev.is_some_and(|p| p.hazard.is_hazardous)),
            class: cat.hazard_class.or(prev.filter(|_| same_cat).and_then(|p| p.hazard.class)),
            handling: cat.handling.clone().or(prev.and_then(|p| p.hazard.handling.clone())),
        },
        evidence: prev.filter(|_| same_cat).map(|p| p.evidence.clone()).unwrap_or_default(),
        source: if same_cat { prev.map(|p| p.source).unwrap_or(ItemSource::Seller) } else { ItemSource::Seller },
        ai_category_id: prev.and_then(|p| p.ai_category_id.clone()),
        ai_confidence: prev.and_then(|p| p.ai_confidence),
        ladder: prev.filter(|_| same_cat).map(|p| p.ladder).unwrap_or(LadderLevel::L0Claimed),
        note: i.note.map(|n| n.chars().take(300).collect()),
    })
}

/// Short human title, e.g. "Copper, Newspaper +2".
pub fn title_for(tax: &Taxonomy, items: &[Item]) -> String {
    let mut names: Vec<String> = Vec::new();
    for i in items {
        let n = tax.get(&i.category_id).map(|c| c.name.clone()).unwrap_or_else(|| i.category_id.clone());
        if !names.contains(&n) {
            names.push(n);
        }
    }
    match names.len() {
        0 => "New listing".into(),
        1 | 2 => names.join(", "),
        n => format!("{}, {} +{}", names[0], names[1], n - 2),
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PresignImage {
    pub kind: ImageKind,
    pub source: CaptureSource,
    pub content_type: String,
    pub size: u64,
    /// Base64 SHA-256 of the file; enforced by S3 on upload.
    pub sha256: Option<String>,
    pub proof_request_id: Option<ProofRequestId>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ImageUpload {
    pub image_id: ImageId,
    pub upload: PresignedUpload,
}

pub async fn presign_image_route(
    State(st): State<AppState>,
    me: Me,
    Path(id): Path<String>,
    Json(req): Json<PresignImage>,
) -> ApiResult<Json<ImageUpload>> {
    let id: ListingId = parse(&id, "listing id")?;
    Ok(Json(presign_image(&st, &me, &id, req).await?))
}

pub async fn skip_proofs_route(State(st): State<AppState>, me: Me, Path(id): Path<String>) -> ApiResult<Json<ListingView>> {
    let id: ListingId = parse(&id, "listing id")?;
    Ok(Json(skip_proofs(&st, &me, &id).await?))
}

#[utoipa::path(post, path = "/v1/listings/{id}/images:presign", tag = "listings", request_body = PresignImage, responses((status = 200, body = ImageUpload)))]
pub async fn presign_image(st: &AppState, me: &Me, id: &ListingId, req: PresignImage) -> ApiResult<ImageUpload> {
    let b = load_bundle(st, id).await?;
    as_seller(st, me, &b, Action::EditListing).await?;
    if !["image/jpeg", "image/png", "image/webp"].contains(&req.content_type.as_str()) {
        return Err(ApiError::bad("photos must be JPEG, PNG or WebP"));
    }
    if req.size == 0 || req.size > 15 * 1024 * 1024 {
        return Err(ApiError::bad("photos must be under 15 MB"));
    }
    if b.images.len() >= MAX_IMAGES {
        return Err(ApiError::bad(format!("at most {MAX_IMAGES} photos per listing")));
    }
    if req.kind == ImageKind::Proof && req.source != CaptureSource::Camera {
        return Err(ApiError::bad("proof photos must be taken with the camera"));
    }
    if let Some(p) = &req.proof_request_id
        && !b.proofs.iter().any(|r| &r.data.id == p)
    {
        return Err(ApiError::NotFound("proof request"));
    }
    let image_id = ImageId::new();
    let ext = req.content_type.rsplit('/').next().unwrap_or("jpg");
    let key = format!("listings/{id}/original/{image_id}.{ext}");
    let upload = st.ix.blobs.presign_put(&st.cfg.media_bucket, &key, &req.content_type, req.size, req.sha256.as_deref()).await?;
    let meta = ImageMeta {
        id: image_id.clone(),
        kind: req.kind,
        source: req.source,
        original_key: key,
        processed_key: None,
        thumb_key: None,
        content_type: req.content_type,
        size_bytes: req.size,
        sha256: req.sha256,
        phash: None,
        triage: None,
        faces_blurred: 0,
        proof_request_id: req.proof_request_id,
        uploaded: false,
        created_at: now(),
    };
    st.db.put_image(id, b.listing.data.current_version, &meta).await?;
    Ok(ImageUpload { image_id, upload })
}

/// `POST /v1/listings/{id}/images/{imageId}:complete` after the S3 upload.
pub async fn image_action(State(st): State<AppState>, me: Me, Path((id, seg)): Path<(String, String)>) -> ApiResult<Json<ImageView>> {
    let id: ListingId = parse(&id, "listing id")?;
    let (img_id, action): (ImageId, String) = id_action(&seg, "image id")?;
    if action != "complete" {
        return Err(ApiError::NotFound("action"));
    }
    let b = load_bundle(&st, &id).await?;
    as_seller(&st, &me, &b, Action::EditListing).await?;
    let ver = b.listing.data.current_version;
    let mut img = st.db.image(&id, ver, &img_id).await?.ok_or(ApiError::NotFound("image"))?;
    if !st.ix.blobs.exists(&st.cfg.media_bucket, &img.data.original_key).await? {
        return Err(ApiError::conflict("upload the file first"));
    }
    img.data.uploaded = true;
    st.db.put_image(&id, ver, &img.data).await?;
    st.tasks.enqueue(Task::ProcessImage { listing_id: id.clone(), ver, image_id: img_id }).await?;
    Ok(Json(image_views(&st, &[img], Viewer::Seller).await?.pop().ok_or(ApiError::NotFound("image"))?))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ContestClaim {
    pub note: Option<String>,
}

/// `POST /v1/listings/{id}/claims/{claimId}:contest`: vendors then see "seller disputes AI assessment".
#[utoipa::path(post, path = "/v1/listings/{id}/claims/{claimId}:contest", tag = "listings", request_body = ContestClaim, responses((status = 200, body = Claim)))]
pub async fn claim_action(
    State(st): State<AppState>,
    me: Me,
    Path((id, seg)): Path<(String, String)>,
    Json(b): Json<ContestClaim>,
) -> ApiResult<Json<Claim>> {
    let id: ListingId = parse(&id, "listing id")?;
    let (claim_id, action): (ClaimId, String) = id_action(&seg, "claim id")?;
    if action != "contest" {
        return Err(ApiError::NotFound("action"));
    }
    let bundle = load_bundle(&st, &id).await?;
    as_seller(&st, &me, &bundle, Action::EditListing).await?;
    let mut c = bundle.claims.iter().find(|c| c.data.id == claim_id).ok_or(ApiError::NotFound("claim"))?.data.clone();
    c.contested = true;
    c.contest_note = b.note.map(|n| n.chars().take(500).collect());
    st.db.write_ops(vec![st.db.claim_op(&id, bundle.listing.data.current_version, &c)?]).await?;
    Ok(Json(c))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ProvideProof {
    pub proof_request_id: ProofRequestId,
    pub image_ids: Vec<ImageId>,
}

/// Attaches camera photos to a proof request and re-runs verification.
#[utoipa::path(post, path = "/v1/listings/{id}/proofs", tag = "listings", request_body = ProvideProof, responses((status = 200, body = ListingView)))]
pub async fn provide_proof(
    State(st): State<AppState>,
    me: Me,
    Path(id): Path<String>,
    Json(b): Json<ProvideProof>,
) -> ApiResult<Json<ListingView>> {
    let id: ListingId = parse(&id, "listing id")?;
    let mut bundle = load_bundle(&st, &id).await?;
    as_seller(&st, &me, &bundle, Action::EditListing).await?;
    let ver = bundle.listing.data.current_version;
    let mut p = bundle.proofs.iter().find(|p| p.data.id == b.proof_request_id).ok_or(ApiError::NotFound("proof request"))?.data.clone();
    if b.image_ids.is_empty() || b.image_ids.len() > 5 {
        return Err(ApiError::bad("attach 1–5 photos"));
    }
    for img in &b.image_ids {
        let i = bundle.images.iter().find(|i| &i.data.id == img).ok_or(ApiError::NotFound("image"))?;
        if !i.data.uploaded || i.data.source != CaptureSource::Camera {
            return Err(ApiError::bad("proof photos must be uploaded camera captures"));
        }
    }
    p.status = ProofStatus::Provided;
    p.image_ids = b.image_ids;
    let ev = DomainEvent::ProofProvided { listing_id: id.clone(), ver, proof_request_id: p.id.clone() };
    let mut extra = vec![st.db.proof_op(&id, ver, &p)?];
    let still_open = bundle.proofs.iter().any(|x| x.data.id != p.id && x.data.status == ProofStatus::Open);
    let l = &mut bundle.listing;
    if !still_open && l.data.status == ListingStatus::NeedsInput {
        l.data.status = l.data.status.transition(ListingStatus::Analyzing)?;
        let mut v = bundle.version.data.clone();
        v.analysis.status = AnalysisStatus::Queued;
        v.analysis.updated_at = now();
        extra.push(st.db.version_op(&Record::new(bundle.version.meta.clone(), v))?);
    }
    l.data.updated_at = now();
    st.db.save_listing_with(l, extra, vec![ev], vec![]).await?;
    if !still_open {
        st.tasks.enqueue(Task::VerifyListing { listing_id: id.clone(), ver }).await?;
    }
    let bundle = load_bundle(&st, &id).await?;
    Ok(Json(build_view(&st, &bundle, Viewer::Seller, None).await?))
}

/// `POST /v1/listings/{id}/proofs:skip`: the seller can always skip to READY.
#[utoipa::path(post, path = "/v1/listings/{id}/proofs:skip", tag = "listings", responses((status = 200, body = ListingView)))]
pub async fn skip_proofs(st: &AppState, me: &Me, id: &ListingId) -> ApiResult<ListingView> {
    let mut b = load_bundle(st, id).await?;
    as_seller(st, me, &b, Action::EditListing).await?;
    let ver = b.listing.data.current_version;
    let mut extra = Vec::new();
    for p in b.proofs.iter().filter(|p| p.data.status == ProofStatus::Open) {
        let mut p = p.data.clone();
        p.status = ProofStatus::Skipped;
        extra.push(st.db.proof_op(id, ver, &p)?);
    }
    if b.listing.data.status == ListingStatus::NeedsInput {
        b.listing.data.status = b.listing.data.status.transition(ListingStatus::Ready)?;
    }
    b.listing.data.updated_at = now();
    st.db.save_listing_with(&mut b.listing, extra, vec![], vec![]).await?;
    let b = load_bundle(st, id).await?;
    build_view(st, &b, Viewer::Seller, None).await
}

#[utoipa::path(get, path = "/v1/listings/{id}/price-suggestion", tag = "listings", responses((status = 200, body = kabados_pricing::suggest::Suggestion)))]
pub async fn price_suggestion(
    State(st): State<AppState>,
    me: Me,
    Path(id): Path<String>,
) -> ApiResult<Json<kabados_pricing::suggest::Suggestion>> {
    let id: ListingId = parse(&id, "listing id")?;
    let b = load_bundle(&st, &id).await?;
    as_seller(&st, &me, &b, Action::ViewListing).await?;
    Ok(Json(crate::matching::price_suggestion(&st, &b).await?))
}

/// `POST /v1/listings/{id}:analyze | :publish | :withdraw`.
#[utoipa::path(post, path = "/v1/listings/{id}:publish", tag = "listings", responses((status = 200, body = ListingView)))]
pub async fn listing_action(State(st): State<AppState>, me: Me, Path(seg): Path<String>) -> ApiResult<Json<ListingView>> {
    let (id, action) = split_action(&seg);
    let id: ListingId = parse(id, "listing id")?;
    let mut b = load_bundle(&st, &id).await?;
    let ver = b.listing.data.current_version;
    match action {
        Some("analyze") => {
            as_seller(&st, &me, &b, Action::EditListing).await?;
            let has_photo = b.images.iter().any(|i| i.data.uploaded);
            if !has_photo && b.version.data.description.is_none() {
                return Err(ApiError::bad("add a photo or a description first"));
            }
            b.listing.data.status = b.listing.data.status.transition(ListingStatus::Analyzing)?;
            let mut v = b.version.data.clone();
            v.analysis.status = AnalysisStatus::Queued;
            v.analysis.updated_at = now();
            let extra = vec![st.db.version_op(&Record::new(b.version.meta.clone(), v))?];
            st.db.save_listing_with(&mut b.listing, extra, vec![], vec![]).await?;
            st.tasks.enqueue(Task::AnalyzeListing { listing_id: id.clone(), ver }).await?;
        }
        Some("publish") => {
            as_seller(&st, &me, &b, Action::PublishListing).await?;
            publish(&st, &mut b).await?;
        }
        Some("withdraw") => {
            as_seller(&st, &me, &b, Action::WithdrawListing).await?;
            b.listing.data.status = b.listing.data.status.transition(ListingStatus::Withdrawn)?;
            b.listing.data.updated_at = now();
            st.db.save_listing(&mut b.listing).await?;
        }
        _ => return Err(ApiError::NotFound("action")),
    }
    let b = load_bundle(&st, &id).await?;
    Ok(Json(build_view(&st, &b, Viewer::Seller, None).await?))
}

async fn publish(st: &AppState, b: &mut ListingBundle) -> ApiResult<()> {
    if b.items.is_empty() {
        return Err(ApiError::bad("add at least one item"));
    }
    if let Some(i) = b.items.iter().find(|i| i.data.est_weight_g.is_none()) {
        return Err(ApiError::bad(format!("add an estimated weight for {}", i.data.category_id)));
    }
    let l = &mut b.listing.data;
    if l.status == ListingStatus::Draft || l.status == ListingStatus::NeedsInput {
        l.status = l.status.transition(ListingStatus::Ready)?;
    }
    l.status = l.status.transition(ListingStatus::Published)?;
    let at = now();
    l.published_version = Some(l.current_version);
    l.published_at = Some(at);
    l.first_look_until = Some(at + Duration::minutes(st.cfg.first_look_minutes));
    l.expires_at = Some(at + Duration::days(LISTING_TTL_DAYS));
    l.updated_at = at;
    let ev = DomainEvent::ListingPublished { listing_id: l.id.clone(), ver: l.current_version };
    st.db.save_listing_with(&mut b.listing, vec![], vec![ev], vec![]).await?;
    Ok(())
}

/// Seller role is implicit for every account.
pub fn require_seller(me: &Me) -> ApiResult<()> {
    if me.has_role(Role::Seller) { Ok(()) } else { Err(ApiError::forbidden("seller access only")) }
}
