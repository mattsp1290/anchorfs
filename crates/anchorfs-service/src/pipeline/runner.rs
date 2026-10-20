//! Response cancellation never cancels an admitted publication. An atomic
//! handshake prevents a timed-out pre-admission request from admitting later.
use crate::{
    app::AppState,
    control::Kind,
    error::ApiError,
    middleware::{RequestStart, TraceId},
};
use anchorfs_core::{ErrorCode, OperationId};
use std::{
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};
use tokio::time::Instant;

#[derive(Default)]
pub struct PhaseCell {
    phase: AtomicU8,
    operation: Mutex<Option<OperationId>>,
}
pub enum Abandonment {
    BeforeAdmission,
    AfterAdmission(Option<OperationId>),
}
impl PhaseCell {
    pub fn start_admission(&self) -> bool {
        self.phase
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    pub fn admitted(&self, operation: OperationId) {
        *self.operation.lock().expect("phase mutex") = Some(operation);
        self.phase.store(2, Ordering::Release);
    }
    pub fn abandon(&self) -> Abandonment {
        if matches!(
            self.phase
                .compare_exchange(0, 3, Ordering::AcqRel, Ordering::Acquire),
            Ok(_) | Err(3)
        ) {
            Abandonment::BeforeAdmission
        } else {
            Abandonment::AfterAdmission(*self.operation.lock().expect("phase mutex"))
        }
    }
}
#[derive(Clone, Copy)]
pub struct Deadline {
    pub backend_by: Instant,
}
impl Deadline {
    pub async fn within<T>(
        &self,
        future: impl Future<Output = T>,
    ) -> Result<T, tokio::time::error::Elapsed> {
        tokio::time::timeout_at(self.backend_by, future).await
    }
}
#[derive(Clone)]
pub struct Db {
    pub pool: sqlx::PgPool,
    lock_timeout: Duration,
    grace_by: Instant,
}
impl Db {
    async fn configure(
        &self,
        mut tx: sqlx::Transaction<'static, sqlx::Postgres>,
    ) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, ApiError> {
        sqlx::query("SELECT set_config('lock_timeout',$1,true)")
            .bind(format!(
                "{}ms",
                self.lock_timeout.as_millis().max(1).min(i32::MAX as u128)
            ))
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }
    pub async fn begin(&self) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, ApiError> {
        self.configure(self.pool.begin().await?).await
    }
    /// Acquisition may retry during bounded grace; statements/publication never retry implicitly.
    pub async fn begin_final(
        &self,
    ) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, ApiError> {
        loop {
            match tokio::time::timeout_at(self.grace_by, self.pool.begin()).await {
                Ok(Ok(tx)) => return self.configure(tx).await,
                Ok(Err(_)) if Instant::now() < self.grace_by && !self.pool.is_closed() => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                _ => return Err(unavailable()),
            }
        }
    }
}
pub enum Outcome<T> {
    Done(T),
    Crashed,
}
pub fn unavailable() -> ApiError {
    let mut error = ApiError::new(ErrorCode::DependencyUnavailable, "");
    error.retry_after_ms = Some(100);
    error
}
fn abandoned(phase: &PhaseCell, kind: Kind, trace: &str) -> ApiError {
    let mut error = unavailable();
    if let Abandonment::AfterAdmission(operation) = phase.abandon()
        && matches!(kind, Kind::Mutation)
    {
        error.code = ErrorCode::OutcomeUnknown;
        error.message = error.code.as_str().into();
        error.operation_id = operation;
    }
    error.trace_id = trace.into();
    error
}
pub async fn run<
    T: Send + 'static,
    F: Future<Output = Result<Outcome<T>, ApiError>> + Send + 'static,
>(
    state: &AppState,
    start: RequestStart,
    trace: TraceId,
    kind: Kind,
    job: impl FnOnce(Db, Deadline, Arc<dyn super::faults::FaultInjector>, Arc<PhaseCell>) -> F
    + Send
    + 'static,
) -> Result<T, ApiError> {
    let backend_by = Instant::from_std(start.0) + state.config.data_call_deadline;
    let response_by = backend_by - Duration::from_millis(200);
    if response_by.saturating_duration_since(Instant::now()) <= Duration::from_millis(100) {
        let mut error = unavailable();
        error.trace_id = trace.0;
        return Err(error);
    }
    let permit = tokio::time::timeout_at(response_by, state.data_permits.clone().acquire_owned())
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())?;
    let db = Db {
        pool: state.pool.clone(),
        lock_timeout: state.config.db_lock_timeout,
        grace_by: backend_by + state.config.db_lock_timeout,
    };
    let phase = Arc::new(PhaseCell::default());
    let job_phase = phase.clone();
    let faults = state.faults.clone();
    let handle = tokio::spawn(async move {
        let _permit = permit;
        job(db, Deadline { backend_by }, faults, job_phase).await
    });
    match tokio::time::timeout_at(response_by, handle).await {
        Ok(Ok(Ok(Outcome::Done(value)))) => Ok(value),
        Ok(Ok(Err(mut error))) => {
            if error.trace_id.is_empty() {
                error.trace_id = trace.0;
            }
            Err(error)
        }
        Ok(Ok(Ok(Outcome::Crashed))) | Ok(Err(_)) | Err(_) => {
            Err(abandoned(&phase, kind, &trace.0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Barrier, atomic::AtomicUsize};
    #[test]
    fn admission_handshake_is_atomic() {
        for _ in 0..100 {
            let phase = Arc::new(PhaseCell::default());
            let barrier = Arc::new(Barrier::new(2));
            let job_phase = phase.clone();
            let job_barrier = barrier.clone();
            let job = std::thread::spawn(move || {
                job_barrier.wait();
                job_phase.start_admission()
            });
            barrier.wait();
            let outcome = phase.abandon();
            let admitted = job.join().unwrap();
            assert_eq!(admitted, matches!(outcome, Abandonment::AfterAdmission(_)));
            assert!(!phase.start_admission());
        }
    }
    fn state() -> AppState {
        let mut config = crate::config::Config::contract_defaults();
        config.data_call_deadline = Duration::from_millis(500);
        config.max_data_ops = 1;
        let backend = Arc::new(anchorfs_backend::s3::S3Backend::from_config(&config.s3).unwrap());
        AppState::production(
            config,
            sqlx::PgPool::connect_lazy("postgres://unused:unused@127.0.0.1:1/unused").unwrap(),
            backend,
        )
    }
    #[tokio::test]
    async fn timeout_before_admission_prevents_later_mutation() {
        let state = state();
        let effect = Arc::new(AtomicUsize::new(0));
        let job_effect = effect.clone();
        let result = run(
            &state,
            RequestStart(std::time::Instant::now()),
            TraceId("test".into()),
            Kind::Mutation,
            move |_, _, _, phase| async move {
                tokio::time::sleep(Duration::from_millis(450)).await;
                if phase.start_admission() {
                    job_effect.fetch_add(1, Ordering::SeqCst);
                }
                Ok(Outcome::Done(()))
            },
        )
        .await
        .unwrap_err();
        assert_eq!(result.code, ErrorCode::DependencyUnavailable);
        let _permit = tokio::time::timeout(Duration::from_secs(2), state.data_permits.acquire())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(effect.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn timeout_after_admission_finishes_and_keeps_its_permit() {
        let state = state();
        let effect = Arc::new(AtomicUsize::new(0));
        let job_effect = effect.clone();
        let operation = OperationId::new();
        let result = run(
            &state,
            RequestStart(std::time::Instant::now()),
            TraceId("test".into()),
            Kind::Mutation,
            move |_, _, _, phase| async move {
                assert!(phase.start_admission());
                phase.admitted(operation);
                tokio::time::sleep(Duration::from_millis(450)).await;
                job_effect.fetch_add(1, Ordering::SeqCst);
                Ok(Outcome::Done(()))
            },
        )
        .await
        .unwrap_err();
        assert_eq!(result.code, ErrorCode::OutcomeUnknown);
        assert_eq!(result.operation_id, Some(operation));
        assert_eq!(state.data_permits.available_permits(), 0);
        let _permit = tokio::time::timeout(Duration::from_secs(2), state.data_permits.acquire())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(effect.load(Ordering::SeqCst), 1);
    }
}
