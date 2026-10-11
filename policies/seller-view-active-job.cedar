// Sellers see the transporter's ETA and live location during the active job only.
permit (
  principal is Kabados::Seller,
  action == Kabados::Action::"ViewJob",
  resource is Kabados::Job
)
when {
  resource.deal.seller == principal &&
  ["ACCEPTED", "EN_ROUTE", "ARRIVED_PICKUP", "LOADED", "IN_TRANSIT", "ARRIVED_YARD"].contains(resource.status)
};
