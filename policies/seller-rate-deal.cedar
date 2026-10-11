// Sellers rate the other parties once goods are delivered.
permit (
  principal is Kabados::Seller,
  action == Kabados::Action::"RateDeal",
  resource is Kabados::Deal
)
when {
  resource.seller == principal &&
  ["DELIVERED", "SETTLED", "CLOSED"].contains(resource.status)
};
