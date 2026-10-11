// Vendors see their deals and receipts.
permit (
  principal is Kabados::Vendor,
  action in [Kabados::Action::"ViewDeal", Kabados::Action::"ViewReceipt"],
  resource is Kabados::Deal
)
when {
  resource.vendor == principal
};
