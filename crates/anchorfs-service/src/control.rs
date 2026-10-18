//! Request budgets bound responses; owned tasks finish publication after client abandonment.
use crate::{
    app::AppState,
    error::{ApiError, database},
    middleware::{RequestStart, TraceId},
};
use anchorfs_core::ErrorCode;
use sqlx::{Postgres, Transaction};
use std::{
    future::Future,
    pin::Pin,
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub enum Kind {
    Mutation,
    Read,
}
#[derive(Clone, Copy)]
pub struct Budget {
    deadline: Instant,
}
impl Budget {
    pub fn remaining(self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }
}
pub type JobFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ApiError>> + Send + 'a>>;
fn unavailable(kind: Kind, trace: &str) -> ApiError {
    let mut error = ApiError::new(
        match kind {
            Kind::Mutation => ErrorCode::OutcomeUnknown,
            Kind::Read => ErrorCode::DependencyUnavailable,
        },
        trace,
    );
    error.retry_after_ms = Some(100);
    error
}
pub async fn run<T: Send + 'static>(
    state: &AppState,
    start: RequestStart,
    trace: TraceId,
    kind: Kind,
    job: impl for<'a> FnOnce(&'a mut Transaction<'static, Postgres>, Budget) -> JobFuture<'a, T>
    + Send
    + 'static,
) -> Result<T, ApiError> {
    let remaining = state
        .config
        .metadata_call_deadline
        .saturating_sub(start.0.elapsed())
        .saturating_sub(Duration::from_millis(200));
    if remaining <= Duration::from_millis(100) {
        return Err(unavailable(Kind::Read, &trace.0));
    }
    let deadline = Instant::now() + remaining;
    let permit = tokio::time::timeout(remaining, state.control_permits.clone().acquire_owned())
        .await
        .map_err(|_| unavailable(Kind::Read, &trace.0))?
        .map_err(|_| unavailable(Kind::Read, &trace.0))?;
    let pool = state.pool.clone();
    let lock_timeout = state
        .config
        .db_lock_timeout
        .as_millis()
        .max(1)
        .min(i32::MAX as u128);
    let job_trace = trace.0.clone();
    let handle = tokio::spawn(async move {
        let _permit = permit;
        let mut tx = pool.begin().await.map_err(|e| database(e, &job_trace))?;
        sqlx::query("SELECT set_config('lock_timeout', $1, true)")
            .bind(format!("{lock_timeout}ms"))
            .execute(&mut *tx)
            .await
            .map_err(|e| database(e, &job_trace))?;
        let result = job(&mut tx, Budget { deadline }).await;
        match result {
            Ok(value) => {
                tx.commit()
                    .await
                    .map_err(|_| unavailable(kind, &job_trace))?;
                Ok(value)
            }
            Err(mut error) => {
                let _ = tx.rollback().await;
                if error.trace_id.is_empty() {
                    error.trace_id = job_trace;
                }
                Err(error)
            }
        }
    });
    await_job(handle, deadline, kind, &trace.0).await
}
async fn await_job<T>(
    handle: tokio::task::JoinHandle<Result<T, ApiError>>,
    deadline: Instant,
    kind: Kind,
    trace: &str,
) -> Result<T, ApiError> {
    // Dropping the JoinHandle detaches the task; it does not cancel the transaction.
    match tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), handle).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) | Err(_) => Err(unavailable(kind, trace)),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    #[tokio::test]
    async fn abandoned_job_finishes_and_keeps_its_permit() {
        let permits = Arc::new(tokio::sync::Semaphore::new(1));
        let permit = permits.clone().acquire_owned().await.unwrap();
        let effect = Arc::new(AtomicUsize::new(0));
        let job_effect = effect.clone();
        let job = tokio::spawn(async move {
            let _permit = permit;
            tokio::time::sleep(Duration::from_millis(300)).await;
            job_effect.store(1, Ordering::SeqCst);
            Ok(())
        });
        let error = await_job(
            job,
            Instant::now() + Duration::from_millis(100),
            Kind::Mutation,
            "trace",
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::OutcomeUnknown);
        assert_eq!(effect.load(Ordering::SeqCst), 0);
        assert!(
            tokio::time::timeout(Duration::from_millis(30), permits.clone().acquire_owned())
                .await
                .is_err()
        );
        let _permit = tokio::time::timeout(Duration::from_secs(1), permits.acquire_owned())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(effect.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn panic_has_unknown_mutation_outcome() {
        let job = tokio::spawn(async {
            panic!("synthetic publication failure");
            #[allow(unreachable_code)]
            Ok(())
        });
        assert_eq!(
            await_job(
                job,
                Instant::now() + Duration::from_secs(1),
                Kind::Mutation,
                "t"
            )
            .await
            .unwrap_err()
            .code,
            ErrorCode::OutcomeUnknown
        );
    }
}
