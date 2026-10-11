// Transporters need approved KYC and must not be suspended for anything
// (sellers let these people into their homes).
forbid (
  principal is Kabados::Transporter,
  action,
  resource
)
unless {
  principal.kycApproved && !principal.suspended
};
