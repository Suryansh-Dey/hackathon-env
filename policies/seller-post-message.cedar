// Sellers post while the listing is live or a deal is open.
permit (
  principal is Kabados::Seller,
  action == Kabados::Action::"PostMessage",
  resource is Kabados::Thread
)
when {
  resource.seller == principal &&
  (resource.dealOpen || !["WITHDRAWN", "BLOCKED"].contains(resource.listingStatus))
};
