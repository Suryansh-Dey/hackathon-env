// After the seller confirms weights the vendor requests a platform payment (Mode A)
// or marks a direct payment as made (Mode B).
permit (
  principal is Kabados::Vendor,
  action in [Kabados::Action::"RequestPayment", Kabados::Action::"MarkPaidDirect"],
  resource is Kabados::Deal
)
when {
  resource.vendor == principal && resource.status == "SELLER_CONFIRMED"
};
