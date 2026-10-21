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
pub struct Budget {
    pub attempts: u32,
    pub deadline: Instant,
}
impl Budget {
    pub fn new(duration: Duration) -> Self {
        Self {
            attempts: 3,
            deadline: Instant::now() + duration,
        }
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

#[cfg(test)]
mod tests {
    use super::*;
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
