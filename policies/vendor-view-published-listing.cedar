// Any approved, non-suspended vendor can view a published listing (approximate location only).
permit (
  principal is Kabados::Vendor,
  action == Kabados::Action::"ViewListing",
  resource is Kabados::Listing
)
when {
  principal.kycApproved && !principal.suspended && resource.status == "PUBLISHED"
};
