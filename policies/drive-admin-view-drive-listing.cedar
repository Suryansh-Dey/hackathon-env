// Society admins see the listings pooled into their drive (approximate location only).
permit (
  principal is Kabados::SocietyAdmin,
  action == Kabados::Action::"ViewListing",
  resource is Kabados::Listing
)
when {
  resource has driveAdmin && resource.driveAdmin == principal
};
