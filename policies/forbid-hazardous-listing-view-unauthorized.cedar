// Hazardous lots are visible only to vendors with a valid hazardous authorization
// (PLAN.md §7.3 "hazardous (if authorized)"). Forbid overrides permit.
forbid (
  principal is Kabados::Vendor,
  action == Kabados::Action::"ViewListing",
  resource is Kabados::Listing
)
when {
  resource.containsHazardous && !principal.hazardousAuthValid
};
