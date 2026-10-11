// Transporter sees the address only while their job is active (PLAN.md §18.2).
permit (
  principal is Kabados::Transporter,
  action == Kabados::Action::"ViewExactAddress",
  resource is Kabados::Listing
)
when {
  resource has assignedTransporter && resource.assignedTransporter == principal &&
  resource has jobStatus && ["ACCEPTED", "EN_ROUTE", "ARRIVED_PICKUP"].contains(resource.jobStatus)
};
