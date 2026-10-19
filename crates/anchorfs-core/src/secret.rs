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

/// Compare fixed-size secret digests without data-dependent early exits.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut difference = 0u8;
    for (&a, &b) in a.iter().zip(b) {
        difference = std::hint::black_box(difference | (a ^ b));
    }
    difference == 0
}
pub fn secret_hash(secret: &str) -> crate::Sha256Digest {
    crate::sha256(secret.as_bytes())
}
#[derive(Debug, Clone)]
pub struct CredentialProof {
    pub id: crate::CredentialId,
    pub secret: Secret<String>,
}
impl std::str::FromStr for CredentialProof {
    type Err = &'static str;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (id, secret) = value.split_once('.').ok_or("invalid credential proof")?;
        if secret.len() != 64
            || !secret
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("invalid credential proof");
        }
        Ok(Self {
            id: crate::CredentialId(id.parse().map_err(|_| "invalid credential proof")?),
            secret: Secret::new(secret.to_owned()),
        })
    }
}
#[cfg(test)]
mod proof_tests {
    use super::*;
    #[test]
    fn digest_and_proof_contract() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert_eq!(
            secret_hash("abc").to_string(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let id = crate::CredentialId::new();
        let token = random_token();
        let proof: CredentialProof = format!("{id}.{}", token.expose()).parse().unwrap();
        assert_eq!(proof.id, id);
        assert!(format!("{proof:?}").contains("[redacted]"));
        assert!(!format!("{proof:?}").contains(token.expose()));
        for bad in [
            "".into(),
            format!("{id}"),
            format!("{id}.{}", "A".repeat(64)),
            format!("{id}.{}", "f".repeat(63)),
            format!("{id}.{}", "g".repeat(64)),
            format!("bad.{}", "a".repeat(64)),
        ] {
            assert!(bad.parse::<CredentialProof>().is_err());
        }
    }
}

/// Decode exactly 32 bytes; accepts either ASCII hex case, never partial input.
pub fn decode_hex_32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut result = [0; 32];
    for (i, byte) in result.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(result)
}
#[cfg(test)]
mod hex_tests {
    use super::*;
    #[test]
    fn strict_hex() {
        assert_eq!(decode_hex_32(&"aF".repeat(32)), Some([175; 32]));
        for invalid in [
            "a".repeat(63),
            "g".repeat(64),
            "é".repeat(32),
            "a".repeat(65),
        ] {
            assert_eq!(decode_hex_32(&invalid), None);
        }
    }
}
