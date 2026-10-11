// Sellers withdraw their own listing unless it is reserved by a deal or already final.
// Reserved listings are released by cancelling the deal instead.
permit (
  principal is Kabados::Seller,
  action == Kabados::Action::"WithdrawListing",
  resource is Kabados::Listing
)
when {
  resource.seller == principal &&
  !["RESERVED", "COMPLETED", "PARTIALLY_COMPLETED", "WITHDRAWN", "BLOCKED"].contains(resource.status)
};
