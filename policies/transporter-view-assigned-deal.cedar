// The assigned transporter sees the deal they are carrying.
permit (
  principal is Kabados::Transporter,
  action == Kabados::Action::"ViewDeal",
  resource is Kabados::Deal
)
when {
  resource has transporter && resource.transporter == principal
};
