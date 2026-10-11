// Vendors read their own threads.
permit (
  principal is Kabados::Vendor,
  action == Kabados::Action::"ViewThread",
  resource is Kabados::Thread
)
when {
  resource.vendor == principal
};
