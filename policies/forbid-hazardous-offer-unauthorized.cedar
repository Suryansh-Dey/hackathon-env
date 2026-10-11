// Hazardous lots: only vendors with a valid authorization. Forbid always overrides permit.
forbid (
  principal is Kabados::Vendor,
  action == Kabados::Action::"MakeOffer",
  resource is Kabados::Listing
)
when {
  resource.containsHazardous && !principal.hazardousAuthValid
};
