// Ops agents (MFA sessions only) see listings, offers and deals for reviews and disputes.
// Exact addresses stay hidden from ops.
permit (
  principal is Kabados::OpsAgent,
  action in [Kabados::Action::"ViewListing", Kabados::Action::"ViewOffers", Kabados::Action::"ViewDeal"],
  resource
)
when {
  principal.mfa
};
