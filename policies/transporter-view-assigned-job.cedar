// The assigned transporter always sees their job (earnings history).
permit (
  principal is Kabados::Transporter,
  action == Kabados::Action::"ViewJob",
  resource is Kabados::Job
)
when {
  resource has transporter && resource.transporter == principal
};
