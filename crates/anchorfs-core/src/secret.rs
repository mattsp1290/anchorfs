use rand::RngCore;
use std::fmt;
/// Deliberately exposes secrets only through an explicit accessor.
#[derive(Clone)]
pub struct Secret<T>(T);
impl<T> Secret<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }
    pub fn expose(&self) -> &T {
        &self.0
    }
}
impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}
impl<T> fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}
pub fn random_token() -> Secret<String> {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    Secret::new(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redacts_and_generates() {
        let secret = Secret::new("sensitive");
        assert_eq!(format!("{secret:?}"), "[redacted]");
        assert_eq!(format!("{secret}"), "[redacted]");
        let a = random_token();
        let b = random_token();
        assert_eq!(a.expose().len(), 64);
        assert_ne!(a.expose(), b.expose());
    }
}
