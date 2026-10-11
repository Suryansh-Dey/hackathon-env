// Vendors withdraw their own active offers.
permit (
  principal is Kabados::Vendor,
  action == Kabados::Action::"WithdrawOffer",
  resource is Kabados::Offer
)
when {
  resource.vendor == principal && resource.status == "ACTIVE"
};
