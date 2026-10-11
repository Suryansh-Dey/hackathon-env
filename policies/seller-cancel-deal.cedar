// Sellers cancel a deal only before goods are weighed.
permit (
  principal is Kabados::Seller,
  action == Kabados::Action::"CancelDeal",
  resource is Kabados::Deal
)
when {
  resource.seller == principal &&
  ["CREATED", "PICKUP_SCHEDULED", "JOB_ASSIGNED", "AT_PICKUP"].contains(resource.status)
};
