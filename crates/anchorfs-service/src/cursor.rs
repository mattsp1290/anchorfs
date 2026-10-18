//! Scoped, authenticated keyset cursor. MAC verification precedes JSON parsing.
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use uuid::Uuid;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ListCursor {
    pub workspace: Uuid,
    pub parent_inode: i64,
    pub last_name: String,
    pub namespace_generation: i64,
    pub expires_unix_ms: i64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CursorError {
    #[error("cursor_invalid")]
    Invalid,
    #[error("cursor_expired")]
    Expired,
    #[error("cursor_scope")]
    Scope,
}
const DOMAIN: &[u8] = b"anchorfs-cursor-v1";
fn mac(payload: &[u8], key: &[u8; 32]) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("32-byte key");
    mac.update(DOMAIN);
    mac.update(payload);
    mac
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(text: &str) -> Result<Vec<u8>, CursorError> {
    if !text.len().is_multiple_of(2) || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(CursorError::Invalid);
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|_| CursorError::Invalid))
        .collect()
}
pub fn encode(cursor: &ListCursor, key: &[u8; 32]) -> String {
    let payload =
        crate::receipt::canonical_json(&serde_json::to_value(cursor).expect("cursor JSON"));
    format!(
        "1{}{}",
        hex(&payload),
        hex(&mac(&payload, key).finalize().into_bytes())
    )
}
pub fn decode(
    text: &str,
    key: &[u8; 32],
    now_unix_ms: i64,
    workspace: Uuid,
    parent_inode: i64,
) -> Result<ListCursor, CursorError> {
    // Paths have bounded names; reject oversized input before allocating decoded buffers.
    if text.len() < 65 || text.len() > 4096 || !text.starts_with('1') || !text.is_ascii() {
        return Err(CursorError::Invalid);
    }
    let split = text.len() - 64;
    let payload = unhex(&text[1..split])?;
    let tag = unhex(&text[split..])?;
    mac(&payload, key)
        .verify_slice(&tag)
        .map_err(|_| CursorError::Invalid)?;
    let cursor: ListCursor = serde_json::from_slice(&payload).map_err(|_| CursorError::Invalid)?;
    if cursor.expires_unix_ms <= now_unix_ms {
        return Err(CursorError::Expired);
    }
    if cursor.workspace != workspace || cursor.parent_inode != parent_inode {
        return Err(CursorError::Scope);
    }
    anchorfs_core::Name::parse(&cursor.last_name).map_err(|_| CursorError::Invalid)?;
    Ok(cursor)
}
impl From<CursorError> for crate::error::ApiError {
    fn from(error: CursorError) -> Self {
        let mut result = Self::new(anchorfs_core::ErrorCode::InvalidInput, "");
        result.message = error.to_string();
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> ListCursor {
        ListCursor {
            workspace: Uuid::new_v4(),
            parent_inode: 1,
            last_name: "é".into(),
            namespace_generation: 3,
            expires_unix_ms: 2000,
        }
    }
    #[test]
    fn roundtrip_expiry_scope_and_key() {
        let c = fixture();
        let key = [7; 32];
        let wire = encode(&c, &key);
        assert_eq!(decode(&wire, &key, 1999, c.workspace, 1), Ok(c.clone()));
        assert_eq!(
            decode(&wire, &key, 2000, c.workspace, 1),
            Err(CursorError::Expired)
        );
        assert_eq!(
            decode(&wire, &key, 1999, Uuid::new_v4(), 1),
            Err(CursorError::Scope)
        );
        assert_eq!(
            decode(&wire, &key, 1999, c.workspace, 2),
            Err(CursorError::Scope)
        );
        assert_eq!(
            decode(&wire, &[8; 32], 1999, c.workspace, 1),
            Err(CursorError::Invalid)
        );
    }
    #[test]
    fn every_byte_and_extensions_are_authenticated() {
        let c = fixture();
        let wire = encode(&c, &[7; 32]);
        for i in 0..wire.len() {
            let mut altered = wire.as_bytes().to_vec();
            altered[i] = if altered[i] == b'0' { b'1' } else { b'0' };
            assert_eq!(
                decode(
                    std::str::from_utf8(&altered).unwrap(),
                    &[7; 32],
                    0,
                    c.workspace,
                    1
                ),
                Err(CursorError::Invalid)
            );
        }
        for extension in ["0", "00", &"0".repeat(64)] {
            assert_eq!(
                decode(&(wire.clone() + extension), &[7; 32], 0, c.workspace, 1),
                Err(CursorError::Invalid)
            );
        }
    }
    #[test]
    fn rejects_authenticated_unknown_fields_and_trailing_bytes() {
        let c = fixture();
        let mut value = serde_json::to_value(&c).unwrap();
        value["extra"] = serde_json::json!(true);
        for payload in [
            serde_json::to_vec(&value).unwrap(),
            [serde_json::to_vec(&c).unwrap(), b" trailing".to_vec()].concat(),
        ] {
            let wire = format!(
                "1{}{}",
                hex(&payload),
                hex(&mac(&payload, &[7; 32]).finalize().into_bytes())
            );
            assert_eq!(
                decode(&wire, &[7; 32], 0, c.workspace, 1),
                Err(CursorError::Invalid)
            );
        }
    }
    #[test]
    fn rfc4231_case_two() {
        let mut mac = Hmac::<Sha256>::new_from_slice(b"Jefe").unwrap();
        mac.update(b"what do ya want for nothing?");
        assert_eq!(
            hex(&mac.finalize().into_bytes()),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }
}
