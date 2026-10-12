//! Safe constants only. No model bytes, credentials or request objects reach observers.
use super::*;
impl OpenAiCompatibleGateway {
    pub(crate) fn complete_once_observed_with_lifecycle(
        &self,
        request: &ModelRequest,
        cancel: &CancelToken,
        observer: StageObserver,
    ) -> Result<ModelResponse, OneShotFailure> {
        self.complete_once_lifecycle(request, cancel, Some(observer))
    }
    pub(super) fn complete_once_lifecycle(
        &self,
        request: &ModelRequest,
        cancel: &CancelToken,
        observer: Option<StageObserver>,
    ) -> Result<ModelResponse, OneShotFailure> {
        if cancel.is_cancelled() {
            return Err(OneShotFailure::BeforeTransport(ModelError::Cancelled));
        }
        if self.profile.max_output_tokens.is_some_and(|limit| limit == 0 || limit > i64::MAX as u64
            || (self.profile.max_context_tokens != 0 && limit >= self.profile.max_context_tokens)) {
            return Err(OneShotFailure::BeforeTransport(ModelError::Protocol("model_output_limit_invalid".into())));
        }
        let body = self.request_body(request);
        let bytes = serde_json::to_vec(&body).map_err(|_| OneShotFailure::BeforeTransport(ModelError::Protocol("model_request_serialization_failed".into())))?;
        if bytes.len() > self.profile.request_budget_bytes() {
            return Err(OneShotFailure::BeforeTransport(ModelError::ContextOverflow("model_request_payload_budget_exceeded".into())));
        }
        let _permit = admission::gate_for(self.profile.local, &self.profile.endpoint)
            .acquire(cancel)
            .map_err(OneShotFailure::BeforeTransport)?;
        if cancel.is_cancelled() {
            return Err(OneShotFailure::BeforeTransport(ModelError::Cancelled));
        }
        self.round(&body, request, cancel, observer)
            .map_err(OneShotFailure::TransportOutcomeUnknown)
    }
}
