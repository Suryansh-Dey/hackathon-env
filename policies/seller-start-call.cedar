// In-app calls only between participants of an open deal (PLAN.md §18.2, §19.3).
permit (
  principal is Kabados::Seller,
  action == Kabados::Action::"StartCall",
  resource is Kabados::Deal
)
when {
  resource.seller == principal &&
  !["SETTLED", "CLOSED", "CANCELLED"].contains(resource.status)
};
