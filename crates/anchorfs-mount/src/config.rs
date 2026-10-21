use crate::MountError;
use anchorfs_client::{
    auth::{Authority, Bearer, SessionAuthority, WorkflowAuthority},
    endpoint::Endpoint,
};
use anchorfs_core::{Secret, SessionId};
use clap::Args;
use std::path::PathBuf;
use uuid::Uuid;

/// Secret values are accepted through files or environment, never argv.
#[derive(Args, Debug, Clone)]
pub struct MountArgs {
    pub mountpoint: PathBuf,
    #[arg(long)]
    pub endpoint: String,
    #[arg(long)]
    pub workspace: Uuid,
    #[arg(long, requires = "session_epoch")]
    pub session_id: Option<Uuid>,
    #[arg(long, requires = "session_id")]
    pub session_epoch: Option<i64>,
    #[arg(long)]
    pub workflow_epoch: Option<i64>,
    #[arg(long)]
    pub credential_file: Option<PathBuf>,
    #[arg(long)]
    pub workflow_token_file: Option<PathBuf>,
    #[arg(long)]
    pub ca_bundle: Option<PathBuf>,
    #[arg(long)]
    pub allow_plaintext: bool,
    #[arg(long)]
    pub allow_other: bool,
    #[arg(long, default_value_t = 4)]
    pub fuse_threads: usize,
    #[arg(long, default_value_t = 1048576)]
    pub max_io_bytes: u32,
    #[arg(long, default_value_t = 300000)]
    pub handle_ttl_hint_ms: u64,
    #[arg(long, default_value_t = 3600000)]
    pub session_ttl_hint_ms: u64,
    #[arg(long, default_value_t = 10000)]
    pub metadata_deadline_ms: u64,
    #[arg(long, default_value_t = 60000)]
    pub data_deadline_ms: u64,
    #[arg(long,default_value="text",value_parser=["text","json"])]
    pub log_format: String,
}
#[derive(Debug, Clone)]
pub struct MountConfig {
    pub args: MountArgs,
    pub endpoint: Endpoint,
    pub authority: Authority,
    pub ca_bundle: Option<Vec<u8>>,
}
impl MountArgs {
    pub fn config(self) -> Result<MountConfig, MountError> {
        let bearer = read_secret(self.credential_file.as_deref(), "ANCHORFS_MOUNT_CREDENTIAL")?
            .ok_or_else(|| {
                MountError::Config(
                    "a credential file or ANCHORFS_MOUNT_CREDENTIAL is required".into(),
                )
            })?;
        let workflow = read_secret(
            self.workflow_token_file.as_deref(),
            "ANCHORFS_MOUNT_WORKFLOW_TOKEN",
        )?;
        self.with_secrets(
            bearer.expose(),
            workflow.as_ref().map(|v| v.expose().as_str()),
        )
    }
    pub fn with_secrets(
        self,
        bearer: &str,
        token: Option<&str>,
    ) -> Result<MountConfig, MountError> {
        let invalid = |s: &str| MountError::Config(s.into());
        if self.session_id.is_some() != self.session_epoch.is_some()
            || self.session_epoch.is_some_and(|n| n < 1)
        {
            return Err(invalid("session id and positive epoch must be paired"));
        }
        if self.workflow_epoch.is_some() != token.is_some()
            || self.workflow_epoch.is_some_and(|n| n < 1)
        {
            return Err(invalid("workflow token and positive epoch must be paired"));
        }
        if token.is_some() && self.session_id.is_none() {
            return Err(invalid(
                "workflow authority requires a pre-minted bound session",
            ));
        }
        if token.is_some_and(|t| anchorfs_core::decode_hex_32(t).is_none()) {
            return Err(invalid("workflow token must be 64 hex characters"));
        }
        if !(4096..=4194304).contains(&self.max_io_bytes) || !(1..=64).contains(&self.fuse_threads)
        {
            return Err(invalid(
                "max-io-bytes must be 4096..=4194304; fuse-threads must be 1..=64",
            ));
        }
        if [
            self.handle_ttl_hint_ms,
            self.session_ttl_hint_ms,
            self.metadata_deadline_ms,
            self.data_deadline_ms,
        ]
        .iter()
        .any(|n| *n < 2 || *n > 86_400_000)
        {
            return Err(invalid("timer hints and deadlines must be 2..=86400000 ms"));
        }
        let endpoint = Endpoint::parse(&self.endpoint, self.allow_plaintext)
            .map_err(|e| invalid(&e.to_string()))?;
        let authority = Authority {
            bearer: Bearer::parse(bearer).map_err(|e| invalid(&e.to_string()))?,
            session: self.session_id.map(|id| SessionAuthority {
                session_id: SessionId(id),
                session_epoch: self.session_epoch.expect("paired epoch"),
            }),
            workflow: token.map(|t| WorkflowAuthority {
                token: Secret::new(t.into()),
                epoch: self.workflow_epoch.expect("paired epoch"),
            }),
        };
        let ca_bundle = self
            .ca_bundle
            .as_ref()
            .map(std::fs::read)
            .transpose()
            .map_err(|_| invalid("cannot read CA bundle"))?;
        Ok(MountConfig {
            args: self,
            endpoint,
            authority,
            ca_bundle,
        })
    }
}
fn read_secret(
    path: Option<&std::path::Path>,
    env_name: &str,
) -> Result<Option<Secret<String>>, MountError> {
    let value = if let Some(path) = path {
        Some(
            std::fs::read_to_string(path)
                .map_err(|_| MountError::Config("cannot read secret file".into()))?,
        )
    } else {
        std::env::var(env_name).ok()
    };
    Ok(value.map(|v| Secret::new(v.trim().to_owned())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        args: MountArgs,
    }
    pub fn args() -> MountArgs {
        TestCli::try_parse_from([
            "test",
            "/tmp/synthetic",
            "--endpoint",
            "http://localhost:1234",
            "--workspace",
            "00000000-0000-4000-8000-000000000001",
            "--allow-plaintext",
        ])
        .unwrap()
        .args
    }
    #[test]
    fn config_rejects_unpaired_authorities_plaintext_and_size_bounds() {
        let bearer = format!("{}.{}", Uuid::new_v4(), "ab".repeat(32));
        assert!(args().with_secrets(&bearer, None).is_ok());
        let mut a = args();
        a.allow_plaintext = false;
        assert!(a.with_secrets(&bearer, None).is_err());
        let mut a = args();
        a.session_id = Some(Uuid::new_v4());
        assert!(a.with_secrets(&bearer, None).is_err());
        let mut a = args();
        a.workflow_epoch = Some(1);
        assert!(a.with_secrets(&bearer, None).is_err());
        for size in [0, 4095, 4194305, u32::MAX] {
            let mut a = args();
            a.max_io_bytes = size;
            assert!(a.with_secrets(&bearer, None).is_err());
        }
        for size in [4096, 4194304] {
            let mut a = args();
            a.max_io_bytes = size;
            assert!(a.with_secrets(&bearer, None).is_ok());
        }
        assert!(
            args()
                .with_secrets(&bearer, Some(&"ab".repeat(32)))
                .is_err()
        );
        assert!(
            TestCli::try_parse_from(["test", "/tmp/synthetic", "--credential", "secret"]).is_err()
        );
        assert!(
            TestCli::try_parse_from(["test", "/tmp/synthetic", "--workflow-token", "secret"])
                .is_err()
        );
    }
    #[test]
    fn credential_file_is_read_and_debug_redacts() {
        let dir = std::env::temp_dir().join(format!("anchorfs-config-{}", Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("credential");
        let bearer = format!("{}.{}", Uuid::new_v4(), "ac".repeat(32));
        std::fs::write(&path, format!("{bearer}\n")).unwrap();
        let mut a = args();
        a.credential_file = Some(path);
        let cfg = a.config().unwrap();
        assert!(!format!("{cfg:?}").contains(&bearer));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
