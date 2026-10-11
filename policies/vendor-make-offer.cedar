// Approved, non-suspended vendors may offer on published listings.
permit (
  principal is Kabados::Vendor,
  action == Kabados::Action::"MakeOffer",
  resource is Kabados::Listing
)
when {
  principal.kycApproved && !principal.suspended && resource.status == "PUBLISHED"
};
