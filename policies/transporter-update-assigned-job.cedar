// The assigned transporter works through the checklist while the job is active.
permit (
  principal is Kabados::Transporter,
  action == Kabados::Action::"UpdateJob",
  resource is Kabados::Job
)
when {
  resource has transporter && resource.transporter == principal &&
  ["ACCEPTED", "EN_ROUTE", "ARRIVED_PICKUP", "LOADED", "IN_TRANSIT", "ARRIVED_YARD", "DELIVERED"].contains(resource.status)
};
