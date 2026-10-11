// Vendors pick a transporter before pickup.
permit (
  principal is Kabados::Vendor,
  action == Kabados::Action::"AssignTransporter",
  resource is Kabados::Deal
)
when {
  resource.vendor == principal &&
  ["CREATED", "PICKUP_SCHEDULED"].contains(resource.status)
};
