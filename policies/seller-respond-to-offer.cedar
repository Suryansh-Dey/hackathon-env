// Sellers accept, counter or reject active offers on their own published listing.
permit (
  principal is Kabados::Seller,
  action in [Kabados::Action::"AcceptOffer", Kabados::Action::"CounterOffer", Kabados::Action::"RejectOffer"],
  resource is Kabados::Offer
)
when {
  resource.listing.seller == principal &&
  resource.listing.status == "PUBLISHED" &&
  resource.status == "ACTIVE"
};
