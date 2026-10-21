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
