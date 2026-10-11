// Transporters in a deal open disputes while the deal is open.
permit (
  principal is Kabados::Transporter,
  action == Kabados::Action::"OpenDispute",
  resource is Kabados::Deal
)
when {
  resource has transporter && resource.transporter == principal &&
  !["SETTLED", "CLOSED", "CANCELLED", "SELLER_NO_SHOW", "PICKUP_NO_SHOW"].contains(resource.status)
};
