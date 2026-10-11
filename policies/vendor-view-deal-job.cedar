// The deal's vendor sees its transport job.
permit (
  principal is Kabados::Vendor,
  action == Kabados::Action::"ViewJob",
  resource is Kabados::Job
)
when {
  resource.deal.vendor == principal
};
