// The vendor confirms receipt at the yard.
permit (
  principal is Kabados::Vendor,
  action == Kabados::Action::"ConfirmDelivery",
  resource is Kabados::Deal
)
when {
  resource.vendor == principal &&
  ["HANDED_OVER", "IN_TRANSIT"].contains(resource.status)
};
