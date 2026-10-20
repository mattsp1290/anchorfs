use anchorfs_core::OperationId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultPoint {
    AfterPeek,
    BeforeGet,
    BeforeReserve,
    AfterReserve,
    BeforeUpload,
    AfterUpload,
    BeforePublish,
    AfterPublish,
    BeforeFinalize,
    BeforeFollowUp,
}

pub enum FaultAction {
    Continue,
    /// Abandon without finalization, leaving durable state for lazy fencing.
    Crash,
    Wait(tokio::sync::watch::Receiver<bool>),
}

pub trait FaultInjector: Send + Sync {
    fn at(&self, point: FaultPoint, operation: Option<OperationId>) -> FaultAction;
}

pub struct NoFaults;
impl FaultInjector for NoFaults {
    fn at(&self, _: FaultPoint, _: Option<OperationId>) -> FaultAction {
        FaultAction::Continue
    }
}

pub async fn hit(
    faults: &dyn FaultInjector,
    point: FaultPoint,
    operation: Option<OperationId>,
) -> bool {
    match faults.at(point, operation) {
        FaultAction::Continue => true,
        FaultAction::Crash => false,
        FaultAction::Wait(mut gate) => {
            while !*gate.borrow_and_update() {
                if gate.changed().await.is_err() {
                    return false;
                }
            }
            true
        }
    }
}
