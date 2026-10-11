//! OpenAPI 3.1 document generated from the handlers (PLAN.md §17.1). The
//! websites' TypeScript client is generated from it.

use crate::routes::*;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

struct BearerAuth;

impl Modify for BearerAuth {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let c = openapi.components.get_or_insert_with(Default::default);
        c.add_security_scheme(
            "cognito",
            SecurityScheme::Http(HttpBuilder::new().scheme(HttpAuthScheme::Bearer).bearer_format("JWT").build()),
        );
    }
}

#[derive(OpenApi)]
#[openapi(
    info(title = "KabadOS API", version = "1.0.0", description = "Verified scrap marketplace API. Every POST needs an `Idempotency-Key` header."),
    modifiers(&BearerAuth),
    security(("cognito" = [])),
    paths(
        identity::get_me, identity::patch_me, identity::apply_role, identity::list_addresses, identity::add_address, identity::delete_address,
        identity::list_payouts, identity::add_payout, identity::list_consents, identity::set_consent, identity::notifications, identity::data_export,
        identity::delete_me, identity::add_document, identity::list_documents, identity::add_authorization, identity::otp_channel,
        identity::autocomplete, identity::geocode, identity::reverse,
        listings::create_listing, listings::my_listings, listings::get_listing, listings::edit_listing, listings::replace_items,
        listings::presign_image, listings::claim_action, listings::provide_proof, listings::skip_proofs, listings::price_suggestion, listings::listing_action,
        offers::listing_offers, offers::my_quote, offers::make_offer, offers::offer_action, offers::vendor_offers,
        vendor::get_rule_sets, vendor::save_draft, vendor::draft_from_text, vendor::bulk_change, vendor::version_action, vendor::feed, vendor::matches,
        logistics::my_transporters, logistics::invite, logistics::create_job, logistics::availability, logistics::device, logistics::transporter_jobs,
        logistics::job_offers, logistics::get_job, logistics::job_action, logistics::job_event, logistics::positions,
        deals::my_deals, deals::get_deal, deals::deal_action, deals::weigh, deals::line_action, deals::open_dispute, deals::rate, deals::get_receipt,
        deals::presign_deal_image, deals::scale_reading,
        payments::payment_request, payments::verify_checkout,
        chat::open_thread, chat::my_threads, chat::messages, chat::post_message,
        calls::start_call, calls::call_action,
        drives::create_drive, drives::my_drives, drives::by_code, drives::get_drive, drives::drive_action, drives::report,
        public::config, public::taxonomy, public::rate_index, public::impact, public::receipt,
        ops::kyc_queue, ops::kyc_decision, ops::dispute_queue, ops::resolve_dispute, ops::deal_evidence, ops::listing_evidence, ops::ai_reviews,
        ops::put_category, ops::user_action, ops::publish_rate_index, ops::config, ops::audit,
    ),
    tags(
        (name = "me"), (name = "auth"), (name = "places"), (name = "listings"), (name = "offers"), (name = "vendor"), (name = "logistics"),
        (name = "deals"), (name = "payments"), (name = "chat"), (name = "calls"), (name = "drives"), (name = "public"), (name = "ops")
    )
)]
pub struct ApiDoc;

pub fn spec() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}
