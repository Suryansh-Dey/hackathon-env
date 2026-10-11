// The assigned transporter enters the handover OTP only after payment.
permit (
  principal is Kabados::Transporter,
  action == Kabados::Action::"Handover",
  resource is Kabados::Deal
)
when {
  resource has transporter && resource.transporter == principal &&
  ["PAID", "PAID_DIRECT"].contains(resource.status)
};
