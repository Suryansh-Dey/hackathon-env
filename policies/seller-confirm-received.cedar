// Mode B: the seller confirms they received the direct payment.
permit (
  principal is Kabados::Seller,
  action == Kabados::Action::"ConfirmReceived",
  resource is Kabados::Deal
)
when {
  resource.seller == principal && resource.status == "SELLER_CONFIRMED"
};
