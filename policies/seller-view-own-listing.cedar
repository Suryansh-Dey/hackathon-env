// Sellers see their own listings and every offer on them.
permit (
  principal is Kabados::Seller,
  action in [Kabados::Action::"ViewListing", Kabados::Action::"ViewOffers"],
  resource is Kabados::Listing
)
when {
  resource.seller == principal
};
