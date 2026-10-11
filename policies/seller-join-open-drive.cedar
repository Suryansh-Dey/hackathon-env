// Any seller joins an open drive via its link or QR code.
permit (
  principal is Kabados::Seller,
  action == Kabados::Action::"JoinDrive",
  resource is Kabados::Drive
)
when {
  resource.status == "OPEN"
};
