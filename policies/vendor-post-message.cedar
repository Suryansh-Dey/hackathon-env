// Non-suspended vendors post while the listing is live or a deal is open.
permit (
  principal is Kabados::Vendor,
  action == Kabados::Action::"PostMessage",
  resource is Kabados::Thread
)
when {
  resource.vendor == principal && !principal.suspended &&
  (resource.dealOpen || !["WITHDRAWN", "BLOCKED"].contains(resource.listingStatus))
};
