// The assigned transporter records scale readings and re-grades lines at pickup.
permit (
  principal is Kabados::Transporter,
  action in [Kabados::Action::"RecordWeighing", Kabados::Action::"RegradeLine"],
  resource is Kabados::Deal
)
when {
  resource has transporter && resource.transporter == principal &&
  ["AT_PICKUP", "WEIGHED", "LINE_REGRADED"].contains(resource.status)
};
