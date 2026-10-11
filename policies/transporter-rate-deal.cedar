// Transporters rate the other parties once goods are delivered.
permit (
  principal is Kabados::Transporter,
  action == Kabados::Action::"RateDeal",
  resource is Kabados::Deal
)
when {
  resource has transporter && resource.transporter == principal &&
  ["DELIVERED", "SETTLED", "CLOSED"].contains(resource.status)
};
