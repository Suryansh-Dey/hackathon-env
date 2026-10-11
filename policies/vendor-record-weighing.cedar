// The vendor records scale readings and re-grades lines at pickup.
permit (
  principal is Kabados::Vendor,
  action in [Kabados::Action::"RecordWeighing", Kabados::Action::"RegradeLine"],
  resource is Kabados::Deal
)
when {
  resource.vendor == principal &&
  ["AT_PICKUP", "WEIGHED", "LINE_REGRADED"].contains(resource.status)
};
