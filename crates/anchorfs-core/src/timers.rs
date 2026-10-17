use std::{collections::BTreeMap, time::Duration};
#[derive(Debug, Clone)]
pub struct Timers {
    pub handle_ttl: Duration,
    pub upload_reservation_ttl: Duration,
    pub reservation_heartbeat: Duration,
    pub client_session_ttl: Duration,
    pub workflow_lease_ttl: Duration,
    pub read_pin_ttl: Duration,
    pub receipt_retention: Duration,
    pub cursor_ttl: Duration,
    pub restart_recovery_bound: Duration,
    pub metadata_call_deadline: Duration,
    pub data_call_deadline: Duration,
    pub shutdown_drain: Duration,
}
impl Default for Timers {
    fn default() -> Self {
        Self {
            handle_ttl: Duration::from_secs(300),
            upload_reservation_ttl: Duration::from_secs(900),
            reservation_heartbeat: Duration::from_secs(60),
            client_session_ttl: Duration::from_secs(3600),
            workflow_lease_ttl: Duration::from_secs(600),
            read_pin_ttl: Duration::from_secs(300),
            receipt_retention: Duration::from_secs(86400),
            cursor_ttl: Duration::from_secs(600),
            restart_recovery_bound: Duration::from_secs(60),
            metadata_call_deadline: Duration::from_secs(10),
            data_call_deadline: Duration::from_secs(60),
            shutdown_drain: Duration::from_secs(10),
        }
    }
}
impl Timers {
    pub fn values(&self) -> BTreeMap<&'static str, u64> {
        BTreeMap::from([
            ("handle_ttl", self.handle_ttl.as_secs()),
            (
                "upload_reservation_ttl",
                self.upload_reservation_ttl.as_secs(),
            ),
            (
                "reservation_heartbeat",
                self.reservation_heartbeat.as_secs(),
            ),
            ("client_session_ttl", self.client_session_ttl.as_secs()),
            ("workflow_lease_ttl", self.workflow_lease_ttl.as_secs()),
            ("read_pin_ttl", self.read_pin_ttl.as_secs()),
            ("receipt_retention", self.receipt_retention.as_secs()),
            ("cursor_ttl", self.cursor_ttl.as_secs()),
            (
                "restart_recovery_bound",
                self.restart_recovery_bound.as_secs(),
            ),
            (
                "metadata_call_deadline",
                self.metadata_call_deadline.as_secs(),
            ),
            ("data_call_deadline", self.data_call_deadline.as_secs()),
            ("shutdown_drain", self.shutdown_drain.as_secs()),
        ])
    }
}
