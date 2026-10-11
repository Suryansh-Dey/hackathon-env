// The handover OTP can be entered only after payment (the seller never hands over goods unpaid).
permit (
  principal is Kabados::Vendor,
  action == Kabados::Action::"Handover",
  resource is Kabados::Deal
)
when {
  resource.vendor == principal &&
  ["PAID", "PAID_DIRECT"].contains(resource.status)
};
