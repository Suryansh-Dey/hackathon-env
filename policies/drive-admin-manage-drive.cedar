// Society admins manage their drives and download drive reports.
permit (
  principal is Kabados::SocietyAdmin,
  action in [Kabados::Action::"ManageDrive", Kabados::Action::"ViewDriveReport"],
  resource is Kabados::Drive
)
when {
  resource.admin == principal
};
