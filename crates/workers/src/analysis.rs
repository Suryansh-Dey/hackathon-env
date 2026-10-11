//! AI and media pipeline worker (PLAN.md §11.3–11.5, §6.1).
//!
//! Consumes `Task`s from the media and analysis queues: image processing
//! (EXIF strip via re-encode, face blur, thumbnail, perceptual hash, triage),
//! listing itemization and claim verification, and KYC document extraction.
//! Reuses the AI agents in `kabados-agents` and the repositories in
//! `kabados-db`; never reimplements pricing or state transitions.

use crate::WorkerCtx;
use chrono::Utc;
use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView};
use kabados_agents::{ImageInput, ItemizeInput, VerifyInput};
use kabados_api::tasks::Task;
use kabados_domain::ids::*;
use kabados_domain::ladder::LadderLevel;
use kabados_domain::listing::*;
use kabados_domain::taxonomy::Taxonomy;
use std::collections::BTreeMap;

const PROCESSED_MAX: u32 = 1600;
const THUMB_MAX: u32 = 320;
const JPEG_QUALITY: u8 = 82;

/// SQS task entry point.
pub async fn handle_task(ctx: &WorkerCtx, task: Task) -> anyhow::Result<()> {
    match task {
        Task::ProcessImage { listing_id, ver, image_id } => process_image(ctx, &listing_id, ver, &image_id).await,
        Task::AnalyzeListing { listing_id, ver } => analyze_listing(ctx, &listing_id, ver).await,
        Task::VerifyListing { listing_id, ver } => verify_listing(ctx, &listing_id, ver).await,
        Task::ProcessDocument { user_id, document_id } => process_document(ctx, &user_id, &document_id).await,
    }
}

fn encode_jpeg(img: &DynamicImage) -> anyhow::Result<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    img.to_rgb8().write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, JPEG_QUALITY))?;
    Ok(buf.into_inner())
}

fn downscale(img: &DynamicImage, max: u32) -> DynamicImage {
    let (w, h) = img.dimensions();
    if w.max(h) <= max {
        return img.clone();
    }
    img.resize(max, max, FilterType::Triangle)
}

/// Blurs each normalized `[left, top, width, height]` region in place.
fn blur_faces(img: &mut DynamicImage, faces: &[[f32; 4]]) -> u32 {
    let (w, h) = img.dimensions();
    let mut blurred = 0;
    for f in faces {
        let x = (f[0].clamp(0.0, 1.0) * w as f32) as u32;
        let y = (f[1].clamp(0.0, 1.0) * h as f32) as u32;
        let fw = ((f[2].clamp(0.0, 1.0) * w as f32) as u32).min(w.saturating_sub(x));
        let fh = ((f[3].clamp(0.0, 1.0) * h as f32) as u32).min(h.saturating_sub(y));
        if fw < 2 || fh < 2 {
            continue;
        }
        let region = img.crop_imm(x, y, fw, fh).blur(12.0);
        image::imageops::overlay(img, &region, x as i64, y as i64);
        blurred += 1;
    }
    blurred
}

/// 64-bit perceptual hash → four 16-bit bands for near-duplicate lookup (§14).
fn phash_bands(bytes: &[u8]) -> (Vec<u8>, Vec<(u8, u16)>) {
    let hash = image_hasher::HasherConfig::new().to_hasher();
    let img = match image::load_from_memory(bytes) {
        Ok(i) => i,
        Err(_) => return (vec![], vec![]),
    };
    let h = hash.hash_image(&img);
    let raw = h.as_bytes().to_vec();
    let mut bands = Vec::new();
    for (i, chunk) in raw.chunks(2).take(4).enumerate() {
        let v = (chunk[0] as u16) << 8 | *chunk.get(1).unwrap_or(&0) as u16;
        bands.push((i as u8, v));
    }
    (raw, bands)
}

async fn process_image(ctx: &WorkerCtx, listing_id: &ListingId, ver: u32, image_id: &ImageId) -> anyhow::Result<()> {
    let Some(rec) = ctx.db.image(listing_id, ver, image_id).await? else {
        tracing::warn!(%image_id, "image meta missing; skipping");
        return Ok(());
    };
    let mut meta = rec.data;
    let Some(listing) = ctx.db.listing(listing_id).await? else { return Ok(()) };
    let original = ctx.ix.blobs.get(&ctx.cfg.media_bucket, &meta.original_key).await?;
    let decoded = image::load_from_memory(&original)?;

    // Faces are detected on the original, blurred on the downscaled copy (EXIF dropped by re-encode).
    let faces = ctx.ix.vision.faces(&original).await.unwrap_or_default();
    let mut processed = downscale(&decoded, PROCESSED_MAX);
    meta.faces_blurred = blur_faces(&mut processed, &faces);
    let processed_bytes = encode_jpeg(&processed)?;
    // From the blurred copy: vendors see thumbnails too.
    let thumb_bytes = encode_jpeg(&downscale(&processed, THUMB_MAX))?;

    // Perceptual hash + cross-seller near-duplicate check.
    let (raw_hash, bands) = phash_bands(&processed_bytes);
    if !bands.is_empty() {
        ctx.db.put_phash_bands(image_id, listing_id, &listing.data.seller_id, &bands).await.ok();
        let candidates = ctx.db.phash_candidates(&bands).await.unwrap_or_default();
        let cross_seller = candidates.iter().any(|c| {
            c.get("data").and_then(|d| d.get("seller_id")).and_then(|v| v.as_str()).is_some_and(|s| s != listing.data.seller_id.as_str())
                && c.get("data").and_then(|d| d.get("image_id")).and_then(|v| v.as_str()) != Some(image_id.as_str())
        });
        if cross_seller {
            flag_for_review(ctx, listing_id, "near-duplicate image also used by another seller").await.ok();
        }
        meta.phash = Some(hex::encode(&raw_hash));
    }

    // Moderation and in-app triage.
    let labels = ctx.ix.vision.moderation_labels(&original).await.unwrap_or_default();
    if !labels.is_empty() {
        flag_for_review(ctx, listing_id, &format!("moderation labels: {}", labels.join(", "))).await.ok();
    }
    let rt = kabados_agents::runtime_from_env();
    let triage_input =
        ImageInput { image_id: image_id.clone(), mime: "image/jpeg".into(), bytes: processed_bytes.clone(), kind: Some(meta.kind) };
    meta.triage = kabados_agents::triage(&rt, &triage_input).await.ok().map(|o| o.value);

    // Store processed + thumbnail; vendors only ever see these.
    let processed_key = format!("listings/{listing_id}/processed/{image_id}.jpg");
    let thumb_key = format!("listings/{listing_id}/thumb/{image_id}.jpg");
    ctx.ix.blobs.put(&ctx.cfg.media_bucket, &processed_key, processed_bytes, "image/jpeg").await?;
    ctx.ix.blobs.put(&ctx.cfg.media_bucket, &thumb_key, thumb_bytes, "image/jpeg").await?;
    meta.processed_key = Some(processed_key);
    meta.thumb_key = Some(thumb_key);
    ctx.db.put_image(listing_id, ver, &meta).await?;
    Ok(())
}

async fn flag_for_review(ctx: &WorkerCtx, listing_id: &ListingId, reason: &str) -> anyhow::Result<()> {
    if let Some(mut l) = ctx.db.listing(listing_id).await? {
        if !l.data.ai_flagged_for_review {
            l.data.ai_flagged_for_review = true;
            ctx.db.save_listing(&mut l).await?;
        }
        ctx.db.audit(listing_id.as_str(), "system", "FLAGGED_FOR_REVIEW", serde_json::json!({ "reason": reason })).await.ok();
    }
    Ok(())
}

/// Gathers processed (vendor-safe) bytes for every usable, uploaded image.
async fn image_inputs(ctx: &WorkerCtx, bundle: &kabados_db::repo::ListingBundle) -> Vec<ImageInput> {
    let mut out = Vec::new();
    for img in bundle.images.iter().map(|r| &r.data).filter(|i| i.uploaded && i.triage.as_ref().is_none_or(|t| t.usable)) {
        let key = img.processed_key.as_ref().unwrap_or(&img.original_key);
        if let Ok(bytes) = ctx.ix.blobs.get(&ctx.cfg.media_bucket, key).await {
            out.push(ImageInput { image_id: img.id.clone(), mime: "image/jpeg".into(), bytes, kind: Some(img.kind) });
        }
    }
    out
}

async fn set_analysis(
    ctx: &WorkerCtx,
    listing_id: &ListingId,
    ver: u32,
    status: AnalysisStatus,
    rounds: u8,
    model: Option<String>,
) -> anyhow::Result<()> {
    if let Some(bundle) = ctx.db.listing_bundle(listing_id, Some(ver)).await? {
        let mut v = bundle.version.data.clone();
        v.analysis = AnalysisState { status, proof_rounds: rounds, model, prompt_version: None, updated_at: Utc::now() };
        let rec = kabados_db::record::Record::new(bundle.version.meta.clone(), v);
        ctx.db.write_ops(vec![ctx.db.version_op(&rec)?]).await?;
    }
    Ok(())
}

async fn analyze_listing(ctx: &WorkerCtx, listing_id: &ListingId, ver: u32) -> anyhow::Result<()> {
    let Some(mut bundle) = ctx.db.listing_bundle(listing_id, Some(ver)).await? else { return Ok(()) };
    if bundle.listing.data.status != ListingStatus::Analyzing {
        return Ok(());
    }
    let tax = ctx.taxonomy().await.map_err(anyhow::Error::msg)?;
    let rt = kabados_agents::runtime_from_env();
    let images = image_inputs(ctx, &bundle).await;
    let description = bundle.version.data.description.clone();

    // Itemize. On failure, fail closed: proceed to READY without AI items, flagged.
    let items: Vec<Item> = match kabados_agents::itemize(
        &rt,
        &ItemizeInput { images: images.clone(), description: description.clone(), taxonomy: &tax },
    )
    .await
    {
        Ok(out) => {
            let existing: BTreeMap<(String, Option<String>), kabados_domain::Grams> = bundle
                .items
                .iter()
                .filter_map(|r| r.data.est_weight_g.map(|w| ((r.data.category_id.clone(), r.data.grade.clone()), w)))
                .collect();
            out.value.items.iter().map(|d| draft_to_item(&tax, d, &existing)).collect()
        }
        Err(e) => {
            tracing::error!(error = %e, "itemization failed; failing closed");
            flag_for_review(ctx, listing_id, "AI itemization failed").await.ok();
            set_analysis(ctx, listing_id, ver, AnalysisStatus::FailedClosed, bundle.version.data.analysis.proof_rounds, None).await?;
            move_to(ctx, listing_id, ListingStatus::Ready).await?;
            return Ok(());
        }
    };

    // Replace items only when the seller hasn't taken over (all current items are AI-sourced).
    let seller_owned = bundle.items.iter().any(|r| r.data.source == ItemSource::Seller);
    if !seller_owned {
        write_items(ctx, listing_id, ver, &bundle, &items).await?;
        bundle = ctx.db.listing_bundle(listing_id, Some(ver)).await?.unwrap_or(bundle);
    }

    run_verification(ctx, &mut bundle, &rt, &tax, &images).await?;
    Ok(())
}

async fn verify_listing(ctx: &WorkerCtx, listing_id: &ListingId, ver: u32) -> anyhow::Result<()> {
    let Some(mut bundle) = ctx.db.listing_bundle(listing_id, Some(ver)).await? else { return Ok(()) };
    let tax = ctx.taxonomy().await.map_err(anyhow::Error::msg)?;
    let rt = kabados_agents::runtime_from_env();
    let images = image_inputs(ctx, &bundle).await;
    run_verification(ctx, &mut bundle, &rt, &tax, &images).await
}

async fn run_verification(
    ctx: &WorkerCtx,
    bundle: &mut kabados_db::repo::ListingBundle,
    rt: &kabados_agents::AgentRuntime,
    tax: &Taxonomy,
    images: &[ImageInput],
) -> anyhow::Result<()> {
    use kabados_domain::state::StateMachine;
    let listing_id = bundle.listing.data.id.clone();
    let ver = bundle.version.data.ver;
    let rounds = bundle.version.data.analysis.proof_rounds;
    let items: Vec<Item> = bundle.items.iter().map(|r| r.data.clone()).collect();
    let input = VerifyInput {
        items: items.clone(),
        images: images.to_vec(),
        description: bundle.version.data.description.clone(),
        asking_price: bundle.version.data.asking_price,
        claim_reason: bundle.version.data.claim_reason.clone(),
        prior_rounds: rounds,
        label_text: BTreeMap::new(),
    };

    let (claims, proofs, status) = match kabados_agents::verify(rt, &input, tax).await {
        Ok(out) => {
            let mut key_to_id: BTreeMap<String, ClaimId> = BTreeMap::new();
            let claims: Vec<Claim> = out
                .value
                .claims
                .iter()
                .map(|c| {
                    let id = ClaimId::new();
                    key_to_id.insert(c.key.clone(), id.clone());
                    Claim {
                        id,
                        item_id: c.item_id.clone(),
                        text: c.text.clone(),
                        text_ref: c.text_ref.clone(),
                        verdict: c.verdict,
                        ladder_level: c.ladder_level,
                        rationale: c.rationale.clone(),
                        evidence: c.evidence.clone(),
                        confidence: c.confidence,
                        contested: false,
                        contest_note: None,
                    }
                })
                .collect();
            let next_round = rounds + 1;
            let proofs: Vec<ProofRequest> = out
                .value
                .proof_requests
                .iter()
                .filter_map(|p| {
                    key_to_id.get(&p.claim_key).map(|cid| ProofRequest {
                        id: ProofRequestId::new(),
                        claim_id: cid.clone(),
                        round: next_round,
                        shot_type: p.shot_type,
                        instruction: p.instruction.clone(),
                        benefit: p.benefit.clone(),
                        status: ProofStatus::Open,
                        image_ids: vec![],
                    })
                })
                .collect();
            let status =
                if !proofs.is_empty() && next_round <= MAX_PROOF_ROUNDS { ListingStatus::NeedsInput } else { ListingStatus::Ready };
            (claims, proofs, status)
        }
        Err(e) => {
            tracing::error!(error = %e, "claim verification failed; failing closed");
            flag_for_review(ctx, &listing_id, "AI claim verification failed").await.ok();
            (vec![], vec![], ListingStatus::Ready)
        }
    };

    // Raise each item's ladder to the best supported claim referencing it.
    let mut ladder: BTreeMap<ItemId, LadderLevel> = BTreeMap::new();
    for c in claims.iter().filter(|c| c.verdict == Verdict::Supported) {
        if let Some(item) = &c.item_id {
            let e = ladder.entry(item.clone()).or_default();
            *e = (*e).max(c.ladder_level);
        }
    }

    let mut ops = Vec::new();
    for c in &claims {
        ops.push(ctx.db.claim_op(&listing_id, ver, c)?);
    }
    for p in &proofs {
        ops.push(ctx.db.proof_op(&listing_id, ver, p)?);
    }
    let mut updated_items = items.clone();
    for item in &mut updated_items {
        if let Some(l) = ladder.get(&item.id) {
            item.ladder = (*l).max(item.ladder);
            ops.push(ctx.db.item_op(&listing_id, ver, item)?);
        }
    }
    if !ops.is_empty() {
        ctx.db.write_ops(ops).await?;
    }

    // Recompute summary and advance the listing with events.
    let (total, cats, hazardous, min_ladder) = summarize(&updated_items);
    let mut l = bundle.listing.clone();
    l.data.total_est_weight_g = total;
    l.data.categories = cats;
    l.data.contains_hazardous = hazardous;
    l.data.min_ladder = min_ladder;
    if l.data.status.can_transition_to(status) {
        l.data.status = status;
    } else if l.data.status == ListingStatus::Analyzing {
        // Analyzing can always reach Ready/NeedsInput per the state machine.
        l.data.status = status;
    }
    l.data.updated_at = Utc::now();

    let mut version = bundle.version.data.clone();
    version.analysis = AnalysisState {
        status: AnalysisStatus::Done,
        proof_rounds: if status == ListingStatus::NeedsInput { rounds + 1 } else { rounds },
        model: None,
        prompt_version: None,
        updated_at: Utc::now(),
    };
    let vrec = kabados_db::record::Record::new(bundle.version.meta.clone(), version);
    let mut events = vec![kabados_domain::events::DomainEvent::ListingAnalyzed {
        listing_id: listing_id.clone(),
        ver,
        needs_input: status == ListingStatus::NeedsInput,
        failed_closed: false,
    }];
    if !proofs.is_empty() {
        events.push(kabados_domain::events::DomainEvent::ProofRequested {
            listing_id: listing_id.clone(),
            ver,
            count: proofs.len() as u32,
        });
    }
    ctx.db.save_listing_with(&mut l, vec![ctx.db.version_op(&vrec)?], events, vec![]).await?;
    Ok(())
}

async fn move_to(ctx: &WorkerCtx, listing_id: &ListingId, status: ListingStatus) -> anyhow::Result<()> {
    if let Some(mut l) = ctx.db.listing(listing_id).await? {
        use kabados_domain::state::StateMachine;
        if l.data.status.can_transition_to(status) {
            l.data.status = status;
            l.data.updated_at = Utc::now();
            ctx.db.save_listing(&mut l).await?;
        }
    }
    Ok(())
}

async fn write_items(
    ctx: &WorkerCtx,
    listing_id: &ListingId,
    ver: u32,
    bundle: &kabados_db::repo::ListingBundle,
    items: &[Item],
) -> anyhow::Result<()> {
    let mut ops: Vec<_> = bundle.items.iter().map(|r| ctx.db.delete_item_op(listing_id, ver, &r.data.id)).collect();
    for i in items {
        ops.push(ctx.db.item_op(listing_id, ver, i)?);
    }
    ctx.db.write_ops(ops).await?;
    Ok(())
}

/// Maps an AI item draft to a domain item, keeping hazard flags from the
/// taxonomy (hazard recall first) and preserving any seller weight by category.
fn draft_to_item(
    tax: &Taxonomy,
    d: &kabados_agents::ItemDraft,
    existing: &BTreeMap<(String, Option<String>), kabados_domain::Grams>,
) -> Item {
    let cat = tax.get(&d.category_id);
    let hazardous = cat.map(|c| c.hazardous).unwrap_or(d.hazard.is_hazardous) || d.hazard.is_hazardous;
    Item {
        id: ItemId::new(),
        category_id: d.category_id.clone(),
        grade: d.grade.clone(),
        attributes: d.attributes.clone(),
        quantity: d.quantity.max(1),
        unit: cat.map(|c| c.unit).unwrap_or(d.unit),
        est_weight_g: existing.get(&(d.category_id.clone(), d.grade.clone())).copied(),
        plausible_weight: d.plausible_weight,
        hazard: Hazard {
            is_hazardous: hazardous,
            class: cat.and_then(|c| c.hazard_class).or(d.hazard.class),
            handling: cat.and_then(|c| c.handling.clone()).or_else(|| d.hazard.handling.clone()),
        },
        evidence: d.evidence.clone(),
        source: ItemSource::Ai,
        ai_category_id: Some(d.category_id.clone()),
        ai_confidence: Some(d.confidence),
        ladder: LadderLevel::L0Claimed,
        note: d.note.clone(),
    }
}

/// KYC document extraction via Textract (PLAN.md §7.1, §8.3).
async fn process_document(ctx: &WorkerCtx, user_id: &UserId, document_id: &DocumentId) -> anyhow::Result<()> {
    use kabados_domain::user::DocumentKind;
    let Some(mut doc) = ctx.db.documents(user_id).await?.into_iter().find(|d| d.data.id == *document_id) else {
        return Ok(());
    };
    let bytes = match ctx.ix.blobs.get(&ctx.cfg.documents_bucket, &doc.data.s3_key).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "document not readable yet");
            return Ok(());
        }
    };
    let queries: &[(&str, &str)] = match doc.data.kind {
        DocumentKind::Pan => &[("number", "What is the PAN number?"), ("name", "What is the name?")],
        DocumentKind::Gstin => &[("number", "What is the GSTIN?"), ("name", "What is the legal name?")],
        DocumentKind::DrivingLicence => &[("number", "What is the licence number?"), ("name", "What is the name?")],
        DocumentKind::VehicleRc => &[("number", "What is the registration number?"), ("name", "What is the owner name?")],
        DocumentKind::SpcbAuthorization | DocumentKind::RecyclerTieUp => {
            &[("number", "What is the authorization number?"), ("validity", "What is the valid until date?")]
        }
        _ => &[],
    };
    if queries.is_empty() {
        return Ok(());
    }
    match ctx.ix.doc_text.queries(&bytes, queries).await {
        Ok(extracted) => {
            doc.data.extracted = extracted;
            ctx.db.put_document(user_id, doc.data.clone()).await?;
        }
        Err(e) => tracing::warn!(error = %e, "textract extraction failed"),
    }
    Ok(())
}
