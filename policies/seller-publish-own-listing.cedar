// Sellers publish (or republish) their own listing from READY, EXPIRED or CANCELLED.
permit (
  principal is Kabados::Seller,
  action == Kabados::Action::"PublishListing",
  resource is Kabados::Listing
)
when {
  resource.seller == principal &&
  ["READY", "EXPIRED", "CANCELLED"].contains(resource.status)
};
