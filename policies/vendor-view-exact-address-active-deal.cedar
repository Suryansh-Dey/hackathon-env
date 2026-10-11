// Exact address only for the vendor of an active deal (PLAN.md §18.2).
permit (
  principal is Kabados::Vendor,
  action == Kabados::Action::"ViewExactAddress",
  resource is Kabados::Listing
)
when {
  resource has acceptedVendor && resource.acceptedVendor == principal &&
  resource has dealStatus &&
  ["CREATED", "PICKUP_SCHEDULED", "JOB_ASSIGNED", "AT_PICKUP", "WEIGHED", "SELLER_CONFIRMED", "AWAITING_PAYMENT", "PAID"].contains(resource.dealStatus)
};
