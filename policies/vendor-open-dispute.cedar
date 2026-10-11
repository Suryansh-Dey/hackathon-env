// Vendors in a deal open disputes while the deal is open.
permit (
  principal is Kabados::Vendor,
  action == Kabados::Action::"OpenDispute",
  resource is Kabados::Deal
)
when {
  resource.vendor == principal &&
  !["SETTLED", "CLOSED", "CANCELLED", "SELLER_NO_SHOW", "PICKUP_NO_SHOW"].contains(resource.status)
};
