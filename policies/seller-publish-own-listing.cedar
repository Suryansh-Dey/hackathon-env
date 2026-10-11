// Sellers publish (or republish) their own listing. DRAFT/NEEDS_INPUT are allowed because a seller may skip AI; the domain publish() enforces the state path.
permit (
  principal is Kabados::Seller,
  action == Kabados::Action::"PublishListing",
  resource is Kabados::Listing
)
when {
  resource.seller == principal &&
  ["DRAFT", "NEEDS_INPUT", "READY", "EXPIRED", "CANCELLED"].contains(resource.status)
};
