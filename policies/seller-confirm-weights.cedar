// The seller confirms (or disputes) the weighed amounts.
permit (
  principal is Kabados::Seller,
  action == Kabados::Action::"ConfirmWeights",
  resource is Kabados::Deal
)
when {
  resource.seller == principal && resource.status == "WEIGHED"
};
