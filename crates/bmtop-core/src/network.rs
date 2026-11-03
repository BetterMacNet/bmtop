//! Socket accounting from Apple's nettop, not packet capture or wire-byte totals.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetworkFlow {
    pub pid: i32,
    pub command: Option<String>,
    pub protocol: String,
    pub local_endpoint: Option<String>,
    pub remote_endpoint: Option<String>,
    pub interface: Option<String>,
    pub state: Option<String>,
    pub received_bytes: Option<u64>,
    pub sent_bytes: Option<u64>,
    pub receive_bytes_per_second: Option<f64>,
    pub send_bytes_per_second: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetworkFlowSnapshot {
    pub flows: Vec<NetworkFlow>,
    pub elapsed_seconds: f64,
    pub truncated: bool,
}

impl crate::PrivacyMode {
    pub fn redact_network_flows(self, snapshot: &mut NetworkFlowSnapshot) {
        if self == Self::Strict {
            snapshot.flows = snapshot
                .flows
                .iter()
                .map(|flow| NetworkFlow {
                    command: None,
                    local_endpoint: None,
                    remote_endpoint: None,
                    ..flow.clone()
                })
                .collect();
        }
    }
}
