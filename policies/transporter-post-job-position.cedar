// Live location is shared only from acceptance to delivery (PLAN.md §8.4).
permit (
  principal is Kabados::Transporter,
  action == Kabados::Action::"PostJobPosition",
  resource is Kabados::Job
)
when {
  resource has transporter && resource.transporter == principal &&
  ["ACCEPTED", "EN_ROUTE", "ARRIVED_PICKUP", "LOADED", "IN_TRANSIT", "ARRIVED_YARD"].contains(resource.status)
};
