use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::Value;

/// Emit recursively sorted object keys independently of serde_json feature unification.
pub fn canonical_json(value: &Value) -> Vec<u8> {
    fn emit(value: &Value, out: &mut Vec<u8>) {
        match value {
            Value::Object(map) => {
                out.push(b'{');
                let mut keys: Vec<_> = map.keys().collect();
                keys.sort_unstable();
                for (i, key) in keys.into_iter().enumerate() {
                    if i != 0 {
                        out.push(b',');
                    }
                    out.extend(serde_json::to_vec(key).expect("JSON key"));
                    out.push(b':');
                    emit(&map[key], out);
                }
                out.push(b'}');
            }
            Value::Array(values) => {
                out.push(b'[');
                for (i, value) in values.iter().enumerate() {
                    if i != 0 {
                        out.push(b',');
                    }
                    emit(value, out);
                }
                out.push(b']');
            }
            _ => out.extend(serde_json::to_vec(value).expect("JSON scalar")),
        }
    }
    let mut out = Vec::new();
    emit(value, &mut out);
    out
}
pub fn request_digest(op_type: &str, path_ids: &[&str], body: Option<&Value>) -> [u8; 32] {
    let mut bytes =
        format!("anchorfs-request-v1\0{op_type}\0{}\0", path_ids.join("/")).into_bytes();
    bytes.extend(canonical_json(body.unwrap_or(&Value::Null)));
    anchorfs_core::sha256(bytes).0
}
pub fn replay(status: StatusCode, result: Value) -> Response {
    (status, Json(result)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_order_is_recursive() {
        let a = serde_json::from_str(r#"{"z":[{"b":2,"a":1}],"a":0}"#).unwrap();
        let b = serde_json::from_str(r#"{"a":0,"z":[{"a":1,"b":2}]}"#).unwrap();
        assert_eq!(canonical_json(&a), br#"{"a":0,"z":[{"a":1,"b":2}]}"#);
        assert_eq!(canonical_json(&a), canonical_json(&b));
        assert_eq!(
            request_digest("op", &["a"], Some(&a)),
            request_digest("op", &["a"], Some(&b))
        );
        assert_ne!(
            request_digest("op", &["a"], Some(&a)),
            request_digest("op", &["b"], Some(&a))
        );
    }
}
