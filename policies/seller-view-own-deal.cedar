// Sellers see their deals and receipts.
permit (
  principal is Kabados::Seller,
  action in [Kabados::Action::"ViewDeal", Kabados::Action::"ViewReceipt"],
  resource is Kabados::Deal
)
when {
  resource.seller == principal
};
