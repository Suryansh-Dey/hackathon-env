// Ops-console actions require a staff session with MFA (PLAN.md §10.1).
permit (
  principal is Kabados::OpsAgent,
  action in [Kabados::Action::"OpsReviewKyc", Kabados::Action::"OpsReviewAi", Kabados::Action::"OpsResolveDispute", Kabados::Action::"OpsManageTaxonomy", Kabados::Action::"OpsManageConfig", Kabados::Action::"OpsViewFraud", Kabados::Action::"OpsPublishRateIndex"],
  resource
)
when {
  principal.mfa
};
