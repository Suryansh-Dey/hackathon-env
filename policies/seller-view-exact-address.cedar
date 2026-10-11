// Sellers always see the exact address of their own listing.
permit (
  principal is Kabados::Seller,
  action == Kabados::Action::"ViewExactAddress",
  resource is Kabados::Listing
)
when {
  resource.seller == principal
};
