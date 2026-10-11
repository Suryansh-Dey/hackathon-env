// The assigned transporter sees the listing's items for their job.
permit (
  principal is Kabados::Transporter,
  action == Kabados::Action::"ViewListing",
  resource is Kabados::Listing
)
when {
  resource has assignedTransporter && resource.assignedTransporter == principal
};
