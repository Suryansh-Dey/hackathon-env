// Vendors cancel a deal only before goods are weighed.
permit (
  principal is Kabados::Vendor,
  action == Kabados::Action::"CancelDeal",
  resource is Kabados::Deal
)
when {
  resource.vendor == principal &&
  ["CREATED", "PICKUP_SCHEDULED", "JOB_ASSIGNED", "AT_PICKUP"].contains(resource.status)
};
