// Sellers edit their own listings while the listing is still editable (mirrors ListingStatus::is_editable).
permit (
  principal is Kabados::Seller,
  action == Kabados::Action::"EditListing",
  resource is Kabados::Listing
)
when {
  resource.seller == principal &&
  ["DRAFT", "NEEDS_INPUT", "READY", "PUBLISHED", "EXPIRED", "CANCELLED"].contains(resource.status)
};
