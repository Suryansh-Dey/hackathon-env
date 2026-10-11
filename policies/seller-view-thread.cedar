// Sellers read their own threads.
permit (
  principal is Kabados::Seller,
  action == Kabados::Action::"ViewThread",
  resource is Kabados::Thread
)
when {
  resource.seller == principal
};
