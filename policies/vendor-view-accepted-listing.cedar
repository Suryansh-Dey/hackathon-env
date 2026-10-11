// The vendor whose offer was accepted keeps access to the listing for the life of the deal.
permit (
  principal is Kabados::Vendor,
  action == Kabados::Action::"ViewListing",
  resource is Kabados::Listing
)
when {
  resource has acceptedVendor && resource.acceptedVendor == principal
};
