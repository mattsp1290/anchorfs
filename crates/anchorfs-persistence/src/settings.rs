use sqlx::{PgPool, Row};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurabilityProfile {
    Durable,
    Development,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingOverride {
    pub value: String,
    pub source: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setting {
    pub setting: String,
    pub reset_val: String,
    pub overrides: Vec<SettingOverride>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurabilitySettings {
    pub synchronous_commit: Setting,
    pub fsync: Setting,
    pub full_page_writes: Setting,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsafeSettings {
    pub setting: String,
    pub value: String,
    pub source: String,
}
impl std::fmt::Display for UnsafeSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unsafe PostgreSQL {}={} ({})",
            self.setting, self.value, self.source
        )
    }
}
impl std::error::Error for UnsafeSettings {}

impl DurabilitySettings {
    pub async fn read(pool: &PgPool) -> Result<Self, sqlx::Error> {
        // Use one connection so effective settings and override identity describe the same session.
        let mut connection = pool.acquire().await?;
        let rows = sqlx::query("SELECT name,setting,reset_val FROM pg_settings WHERE name IN ('synchronous_commit','fsync','full_page_writes')")
            .fetch_all(&mut *connection).await?;
        let overrides = sqlx::query("SELECT split_part(v,'=',1) AS name, substring(v FROM position('=' IN v)+1) AS value, CASE WHEN s.setdatabase<>0 AND s.setrole<>0 THEN 'database-role' WHEN s.setrole<>0 THEN 'role' ELSE 'database' END AS source FROM pg_db_role_setting s CROSS JOIN LATERAL unnest(s.setconfig) v WHERE (s.setdatabase=0 OR s.setdatabase=(SELECT oid FROM pg_database WHERE datname=current_database())) AND (s.setrole=0 OR s.setrole=(SELECT oid FROM pg_roles WHERE rolname=current_user))")
            .fetch_all(&mut *connection).await?;
        let get = |name: &str| -> Result<Setting, sqlx::Error> {
            let row = rows
                .iter()
                .find(|r| r.get::<String, _>("name") == name)
                .ok_or(sqlx::Error::RowNotFound)?;
            Ok(Setting {
                setting: row.try_get("setting")?,
                reset_val: row.try_get("reset_val")?,
                overrides: overrides
                    .iter()
                    .filter(|r| r.get::<String, _>("name") == name)
                    .map(|r| {
                        Ok(SettingOverride {
                            value: r.try_get("value")?,
                            source: r.try_get("source")?,
                        })
                    })
                    .collect::<Result<_, sqlx::Error>>()?,
            })
        };
        Ok(Self {
            synchronous_commit: get("synchronous_commit")?,
            fsync: get("fsync")?,
            full_page_writes: get("full_page_writes")?,
        })
    }
    pub fn unsafe_settings(&self) -> Vec<UnsafeSettings> {
        let mut errors = Vec::new();
        for (name, s) in [
            ("synchronous_commit", &self.synchronous_commit),
            ("fsync", &self.fsync),
            ("full_page_writes", &self.full_page_writes),
        ] {
            // Report one diagnostic per setting, preferring the most specific source.
            let unsafe_value = s
                .overrides
                .iter()
                .find(|v| v.value != "on")
                .map(|v| (v.value.clone(), v.source.clone()))
                .or_else(|| (s.reset_val != "on").then(|| (s.reset_val.clone(), "server".into())))
                .or_else(|| (s.setting != "on").then(|| (s.setting.clone(), "session".into())));
            if let Some((value, source)) = unsafe_value {
                errors.push(UnsafeSettings {
                    setting: name.into(),
                    value,
                    source,
                });
            }
        }
        errors
    }
    pub fn check(&self, profile: DurabilityProfile) -> Result<(), UnsafeSettings> {
        if profile == DurabilityProfile::Development {
            return Ok(());
        }
        self.unsafe_settings()
            .into_iter()
            .next()
            .map_or(Ok(()), Err)
    }
}
#[derive(Debug, thiserror::Error)]
#[error("PostgreSQL encoding must be UTF8, found {0}")]
pub struct EncodingError(pub String);
pub async fn read_server_encoding(pool: &PgPool) -> Result<String, sqlx::Error> {
    sqlx::query_scalar("SHOW server_encoding")
        .fetch_one(pool)
        .await
}
pub fn check_encoding(value: &str) -> Result<(), EncodingError> {
    if value == "UTF8" {
        Ok(())
    } else {
        Err(EncodingError(value.into()))
    }
}
