use crate::error::ClientError;
use anchorfs_core::{CredentialId, CredentialProof, Secret, SessionId};
use reqwest::RequestBuilder;

#[derive(Debug, Clone)]
pub struct Bearer {
    pub credential_id: CredentialId,
    value: Secret<String>,
}
impl Bearer {
    pub fn parse(value: &str) -> Result<Self, ClientError> {
        let proof: CredentialProof = value
            .parse()
            .map_err(|_| ClientError::InvalidInput("invalid bearer credential".into()))?;
        Ok(Self {
            credential_id: proof.id,
            value: Secret::new(value.to_owned()),
        })
    }
}
#[derive(Debug, Clone)]
pub struct SessionAuthority {
    pub session_id: SessionId,
    pub session_epoch: i64,
}
#[derive(Debug, Clone)]
pub struct WorkflowAuthority {
    pub token: Secret<String>,
    pub epoch: i64,
}
#[derive(Debug, Clone)]
pub struct Authority {
    pub bearer: Bearer,
    pub session: Option<SessionAuthority>,
    pub workflow: Option<WorkflowAuthority>,
}
impl Authority {
    pub fn apply(&self, request: RequestBuilder) -> RequestBuilder {
        let mut request = request.bearer_auth(self.bearer.value.expose());
        if let Some(s) = &self.session {
            request = request
                .header("X-Anchorfs-Session-Id", s.session_id.to_string())
                .header("X-Anchorfs-Session-Epoch", s.session_epoch);
        }
        if let Some(w) = &self.workflow {
            request = request
                .header("X-Anchorfs-Workflow-Token", w.token.expose())
                .header("X-Anchorfs-Workflow-Epoch", w.epoch);
        }
        request
    }
}
