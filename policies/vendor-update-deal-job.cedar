// The deal's vendor re-offers, cancels or changes the fare before the goods are loaded.
permit (
  principal is Kabados::Vendor,
  action == Kabados::Action::"UpdateJob",
  resource is Kabados::Job
)
when {
  resource.deal.vendor == principal &&
  ["OFFERED", "ACCEPTED", "EN_ROUTE", "ARRIVED_PICKUP"].contains(resource.status)
};
