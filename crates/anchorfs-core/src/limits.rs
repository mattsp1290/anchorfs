use std::collections::BTreeMap;
#[derive(Debug, Clone)]
pub struct Limits {
    pub chunk_size: u32,
    pub max_api_write: u32,
    pub max_replace_bytes: u64,
    pub max_chunks_per_file: u32,
    pub max_file_size: u64,
    pub max_name_bytes: u32,
    pub max_path_bytes: u32,
    pub max_list_page: u32,
    pub max_handles_per_session: u32,
    pub max_sessions_per_credential: u32,
    pub max_read_pins_per_session: u32,
    pub max_retry_attempts: u32,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            chunk_size: 4194304,
            max_api_write: 4194304,
            max_replace_bytes: 268435456,
            max_chunks_per_file: 1048576,
            max_file_size: 4398046511104,
            max_name_bytes: 255,
            max_path_bytes: 4096,
            max_list_page: 1000,
            max_handles_per_session: 4096,
            max_sessions_per_credential: 64,
            max_read_pins_per_session: 64,
            max_retry_attempts: 3,
        }
    }
}
pub fn checked_file_size(chunk_size: u64, chunks: u64) -> Result<u64, &'static str> {
    chunk_size
        .checked_mul(chunks)
        .filter(|size| *size <= i64::MAX as u64)
        .ok_or("file size exceeds bigint column bound")
}
impl Limits {
    pub fn values(&self) -> BTreeMap<&'static str, u64> {
        BTreeMap::from([
            ("chunk_size", u64::from(self.chunk_size)),
            ("max_api_write", u64::from(self.max_api_write)),
            ("max_replace_bytes", self.max_replace_bytes),
            ("max_chunks_per_file", u64::from(self.max_chunks_per_file)),
            ("max_file_size", self.max_file_size),
            ("max_name_bytes", u64::from(self.max_name_bytes)),
            ("max_path_bytes", u64::from(self.max_path_bytes)),
            ("max_list_page", u64::from(self.max_list_page)),
            (
                "max_handles_per_session",
                u64::from(self.max_handles_per_session),
            ),
            (
                "max_sessions_per_credential",
                u64::from(self.max_sessions_per_credential),
            ),
            (
                "max_read_pins_per_session",
                u64::from(self.max_read_pins_per_session),
            ),
            ("max_retry_attempts", u64::from(self.max_retry_attempts)),
        ])
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.values().values().any(|v| *v == 0) {
            return Err("limits must be positive");
        }
        if self.chunk_size > i32::MAX as u32 || self.max_chunks_per_file > i32::MAX as u32 {
            return Err("chunk limit exceeds integer column bound");
        }
        let capacity = checked_file_size(self.chunk_size.into(), self.max_chunks_per_file.into())?;
        if self.max_file_size > capacity || self.max_file_size > i64::MAX as u64 {
            return Err("file size exceeds bigint column bound or chunk capacity");
        }
        if self.max_replace_bytes > self.max_file_size {
            return Err("replace bound exceeds file size");
        }
        if self.max_api_write > self.chunk_size {
            return Err("API write exceeds chunk size");
        }
        if self.max_name_bytes > 255 {
            return Err("name bound exceeds schema limit");
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limits_bound_column_types() {
        let d = Limits::default();
        assert!(d.validate().is_ok());
        assert_eq!(d.max_file_size, 4 * 1024u64.pow(4));
        assert!(
            Limits {
                chunk_size: 0,
                ..d.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            Limits {
                chunk_size: i32::MAX as u32 + 1,
                ..d.clone()
            }
            .validate()
            .unwrap_err()
            .contains("integer")
        );
        assert!(
            Limits {
                max_chunks_per_file: u32::MAX,
                ..d
            }
            .validate()
            .unwrap_err()
            .contains("integer")
        );
        assert!(checked_file_size(i32::MAX as u64, i32::MAX as u64).is_ok());
        assert!(checked_file_size(u64::MAX, 2).is_err());
        assert!(checked_file_size(i64::MAX as u64, 2).is_err());
    }
}
