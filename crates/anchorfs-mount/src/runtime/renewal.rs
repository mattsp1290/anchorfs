use super::{Mount, lock};
use crate::errno::CallClass;
use anchorfs_client::{error::ClientError, retry::IdempotencyKey};
use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
    time::{Duration, Instant},
};

struct Backoff {
    next: Duration,
    cap: Duration,
}
impl Backoff {
    fn new(ttl: Duration) -> Self {
        Self {
            next: Duration::from_millis(250),
            cap: (ttl / 8).clamp(Duration::from_millis(250), Duration::from_secs(5)),
        }
    }
    fn failed(&mut self, error: &ClientError) -> Duration {
        let hint = match error {
            ClientError::Api(e) => e.retry_after_ms.map(Duration::from_millis),
            _ => None,
        };
        let delay = hint.unwrap_or(self.next).max(self.next);
        self.next = (self.next * 2).min(self.cap);
        delay
    }
}
struct Retry {
    due: Instant,
    backoff: Backoff,
}

pub(super) fn spawn(mount: &Arc<Mount>) {
    mount
        .rt
        .spawn(session(Arc::downgrade(mount), mount.cfg.session_ttl));
    mount.rt.spawn(handles(
        Arc::downgrade(mount),
        mount.cfg.handle_ttl,
        mount.changed.clone(),
    ));
}

async fn session(weak: Weak<Mount>, ttl: Duration) {
    let mut due = Instant::now() + ttl / 2;
    let mut backoff = Backoff::new(ttl);
    loop {
        tokio::time::sleep_until(due.into()).await;
        let Some(mount) = weak.upgrade() else { return };
        if mount.stopped() {
            return;
        }
        let session = mount.client.authority().session.expect("session");
        match mount
            .client
            .renew_session(session.session_id.0, &IdempotencyKey::new())
            .await
        {
            Ok(_) => {
                due = Instant::now() + ttl / 2;
                backoff = Backoff::new(ttl);
            }
            Err(error) => {
                due = Instant::now() + backoff.failed(&error);
                mount.api_error(error, CallClass::Read, None).await;
            }
        }
    }
}

async fn handles(weak: Weak<Mount>, ttl: Duration, changed: Arc<tokio::sync::Notify>) {
    let mut retries = BTreeMap::<u64, Retry>::new();
    loop {
        let Some(mount) = weak.upgrade() else { return };
        if mount.stopped() {
            return;
        }
        let now = Instant::now();
        let (ready, next) = {
            let files = lock(&mount.files);
            retries.retain(|fh, _| files.entries.contains_key(fh));
            let due: Vec<_> = files
                .entries
                .iter()
                .filter(|(_, f)| !f.fenced)
                .map(|(fh, f)| {
                    (
                        *fh,
                        f.handle_id,
                        retries
                            .get(fh)
                            .map_or(f.renewed + ttl / 2, |retry| retry.due),
                    )
                })
                .collect();
            (
                due.iter()
                    .filter(|(_, _, at)| *at <= now)
                    .take(8)
                    .map(|(fh, id, _)| (*fh, *id))
                    .collect::<Vec<_>>(),
                due.iter().map(|(_, _, at)| *at).min(),
            )
        };
        if ready.is_empty() {
            drop(mount);
            match next {
                Some(at) => {
                    tokio::select! {
                        _ = tokio::time::sleep_until(at.into()) => {},
                        _ = changed.notified() => {},
                    }
                }
                None => changed.notified().await,
            }
            continue;
        }
        let mut jobs = tokio::task::JoinSet::new();
        for (fh, id) in ready {
            let mount = mount.clone();
            jobs.spawn(async move {
                (
                    fh,
                    mount.client.renew_handle(id, &IdempotencyKey::new()).await,
                )
            });
        }
        while let Some(result) = jobs.join_next().await {
            let Ok((fh, result)) = result else { continue };
            match result {
                Ok(_) => {
                    retries.remove(&fh);
                    if let Some(file) = lock(&mount.files).entries.get_mut(&fh) {
                        file.renewed = Instant::now();
                    }
                }
                Err(error) => {
                    let retry = retries.entry(fh).or_insert_with(|| Retry {
                        due: now,
                        backoff: Backoff::new(ttl),
                    });
                    retry.due = Instant::now() + retry.backoff.failed(&error);
                    mount.api_error(error, CallClass::Read, Some(fh)).await;
                }
            }
        }
    }
}
