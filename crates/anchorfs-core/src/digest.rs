use sha2::{Digest, Sha256};
use std::{fmt, str::FromStr};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sha256Digest(pub [u8; 32]);
pub fn sha256(bytes: impl AsRef<[u8]>) -> Sha256Digest {
    Sha256Digest(Sha256::digest(bytes.as_ref()).into())
}
impl fmt::Display for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}
impl FromStr for Sha256Digest {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 64 || !s.is_ascii() {
            return Err("invalid SHA-256 hex");
        }
        let mut out = [0; 32];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| "invalid SHA-256 hex")?;
        }
        Ok(Self(out))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_digest() {
        let digest = sha256(b"abc");
        assert_eq!(
            digest.to_string(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(digest.to_string().parse(), Ok(digest));
        assert!("é".repeat(32).parse::<Sha256Digest>().is_err());
    }
}
