// A user acting as a vendor may not offer on a listing they sold themselves.
forbid (
  principal is Kabados::Vendor,
  action == Kabados::Action::"MakeOffer",
  resource is Kabados::Listing
)
when {
  resource.sellerId == principal.userId
};
