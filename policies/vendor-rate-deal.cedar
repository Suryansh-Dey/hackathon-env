// Vendors rate the other parties once goods are delivered.
permit (
  principal is Kabados::Vendor,
  action == Kabados::Action::"RateDeal",
  resource is Kabados::Deal
)
when {
  resource.vendor == principal &&
  ["DELIVERED", "SETTLED", "CLOSED"].contains(resource.status)
};
