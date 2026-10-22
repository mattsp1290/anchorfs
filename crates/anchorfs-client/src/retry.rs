use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct IdempotencyKey(String);
impl IdempotencyKey {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl Default for IdempotencyKey {
    fn default() -> Self {
        Self::new()
    }
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct Budget {
    pub(crate) attempts: u32,
    pub(crate) deadline: Instant,
}
impl Budget {
    pub(crate) fn new(duration: Duration) -> Result<Self, crate::error::ClientError> {
        Ok(Self {
            attempts: 3,
            deadline: Instant::now().checked_add(duration).ok_or_else(|| {
                crate::error::ClientError::InvalidInput("deadline exceeds clock range".into())
            })?,
        })
    }
}
#[derive(Debug, Clone, Copy)]
pub enum Resolver {
    Workspace,
    Admin,
}

impl std::str::FromStr for IdempotencyKey {
    type Err = crate::error::ClientError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() || value.len() > 256 || !value.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(crate::error::ClientError::InvalidInput(
                "idempotency key must contain 1..=256 visible ASCII bytes".into(),
            ));
        }
        Ok(Self(value.to_owned()))
    }
}

/// Bounded transport retry. Once any keyed attempt was uncertain, a later gate
/// cannot establish non-publication: pass the actual error as an ambiguous cause.
pub(crate) async fn execute(
    client: &crate::Client,
    route: &crate::ops::Route,
    key: Option<&IdempotencyKey>,
) -> Result<crate::ops::Response, crate::error::ClientError> {
    use crate::error::ClientError;
    use anchorfs_core::ErrorCode;
    let timeout = route.timeout(client).saturating_add(Duration::from_secs(1));
    let budget = Budget::new(timeout.saturating_mul(3))?;
    let mut last = ClientError::Deadline;
    let mut ambiguous = false;
    for attempt in 0..budget.attempts {
        let remaining = budget.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match client.once(route, key, timeout.min(remaining)).await {
            Ok(value) => return Ok(value),
            Err(error) => {
                let uncertain = matches!(error, ClientError::Transport(_) | ClientError::Deadline)
                    || error.kind() == Some(ErrorCode::OutcomeUnknown);
                ambiguous |= uncertain;
                let retry = matches!(error, ClientError::Transport(_) | ClientError::Deadline)
                    || error.kind() == Some(ErrorCode::DependencyUnavailable)
                    || (key.is_some() && error.kind() == Some(ErrorCode::OutcomeUnknown));
                if !retry {
                    return Err(if key.is_some() && ambiguous {
                        ClientError::Ambiguous(Box::new(error))
                    } else {
                        error
                    });
                }
                let delay = match &error {
                    ClientError::Api(e) => e.retry_after_ms.unwrap_or(100),
                    _ => 100,
                };
                last = error;
                if attempt + 1 < budget.attempts {
                    tokio::time::sleep(
                        Duration::from_millis(delay)
                            .min(budget.deadline.saturating_duration_since(Instant::now())),
                    )
                    .await;
                }
            }
        }
    }
    Err(if key.is_some() && ambiguous {
        ClientError::Ambiguous(Box::new(last))
    } else {
        last
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_unrepresentable_deadline_is_rejected_without_panicking() {
        assert!(matches!(
            Budget::new(Duration::MAX),
            Err(crate::error::ClientError::InvalidInput(_))
        ));
    }
    #[test]
    fn persisted_keys_round_trip_without_changing_identity() {
        let original = IdempotencyKey::new();
        let restored: IdempotencyKey = original.as_str().parse().unwrap();
        assert_eq!(restored.as_str(), original.as_str());
        assert!("k".repeat(256).parse::<IdempotencyKey>().is_ok());
        for invalid in [
            "".into(),
            "k".repeat(257),
            "a b".into(),
            "key\r\nheader:bad".into(),
            "é".into(),
        ] {
            assert!(invalid.parse::<IdempotencyKey>().is_err());
        }
    }
}
