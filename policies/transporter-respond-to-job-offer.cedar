// Transporters a job was offered to may view, accept or decline it while it is open.
permit (
  principal is Kabados::Transporter,
  action in [Kabados::Action::"ViewJob", Kabados::Action::"AcceptJob", Kabados::Action::"DeclineJob"],
  resource is Kabados::Job
)
when {
  resource.offeredTo.contains(principal) && resource.status == "OFFERED"
};
