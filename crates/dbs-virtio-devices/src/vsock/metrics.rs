// Copyright 2022 Alibaba Cloud. All Rights Reserved.
//

use std::fmt::{Debug, Formatter};

use serde::Serialize;

use dbs_utils::metric::{IncMetric, SharedIncMetric};

/// Vsock-related metrics.
#[derive(Default, Serialize)]
pub struct VsockDeviceMetrics {
    /// Number of times when handling events on a vsock device.
    pub event_count: SharedIncMetric,
    /// Number of times when activate failed on a vsock device.
    pub activate_fails: SharedIncMetric,
    /// Number of times when interacting with the space config of a vsock device failed.
    pub cfg_fails: SharedIncMetric,
    /// Number of times when handling RX queue events on a vsock device failed.
    pub rx_queue_event_fails: SharedIncMetric,
    /// Number of times when handling TX queue events on a vsock device failed.
    pub tx_queue_event_fails: SharedIncMetric,
    /// Number of times when handling event queue events on a vsock device failed.
    pub ev_queue_event_fails: SharedIncMetric,
    /// Number of times when handling muxer events on a vsock device failed.
    pub muxer_event_fails: SharedIncMetric,
    /// Number of times when handling connection events on a vsock device failed.
    pub conn_event_fails: SharedIncMetric,
    /// Number of events associated with the receiving queue.
    pub rx_queue_event_count: SharedIncMetric,
    /// Number of events associated with the transmitting queue.
    pub tx_queue_event_count: SharedIncMetric,
    /// Number of events associated with the event queue.
    pub ev_queue_event_count: SharedIncMetric,
    /// Number of events associated with the backend.
    pub backend_event_count: SharedIncMetric,
    /// Number of bytes received.
    pub rx_bytes_count: SharedIncMetric,
    /// Number of transmitted bytes.
    pub tx_bytes_count: SharedIncMetric,
    /// Number of packets received.
    pub rx_packets_count: SharedIncMetric,
    /// Number of transmitted packets.
    pub tx_packets_count: SharedIncMetric,
    /// Number of added connections.
    pub conns_added: SharedIncMetric,
    /// Number of killed connections.
    pub conns_killed: SharedIncMetric,
    /// Number of removed connections.
    pub conns_removed: SharedIncMetric,
    /// How many times the killq has been resynced.
    pub killq_resync: SharedIncMetric,
    /// How many flush fails have been seen.
    pub tx_flush_fails: SharedIncMetric,
    /// How many write fails have been seen.
    pub tx_write_fails: SharedIncMetric,
    /// Number of times read() has failed.
    pub rx_read_fails: SharedIncMetric,
}

impl Debug for VsockDeviceMetrics {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VsockDeviceMetrics")
            .field("activate_fails", &self.activate_fails.count())
            .field("cfg_fails", &self.cfg_fails.count())
            .field("rx_queue_event_fails", &self.rx_queue_event_fails.count())
            .field("tx_queue_event_fails", &self.tx_queue_event_fails.count())
            .field("ev_queue_event_fails", &self.ev_queue_event_fails.count())
            .field("muxer_event_fails", &self.muxer_event_fails.count())
            .field("conn_event_fails", &self.conn_event_fails.count())
            .field("rx_queue_event_count", &self.rx_queue_event_count.count())
            .field("tx_queue_event_count", &self.tx_queue_event_count.count())
            .field("rx_bytes_count", &self.rx_bytes_count.count())
            .field("tx_bytes_count", &self.tx_bytes_count.count())
            .field("rx_packets_count", &self.rx_packets_count.count())
            .field("tx_packets_count", &self.tx_packets_count.count())
            .field("conns_added", &self.conns_added.count())
            .field("conns_killed", &self.conns_killed.count())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use dbs_utils::metric::IncMetric;

    use crate::vsock::metrics::VsockDeviceMetrics;

    #[derive(Default, Debug)]
    pub struct TestDeviceMetrics {
        vsock: Arc<VsockDeviceMetrics>,
    }

    impl TestDeviceMetrics {
        fn new(metrics: Arc<VsockDeviceMetrics>) -> Self {
            Self { vsock: metrics }
        }
    }

    #[test]
    fn test_get_vsock_metrics() {
        let vsock_metrics = Arc::new(VsockDeviceMetrics::default());
        vsock_metrics.activate_fails.inc();
        let metrics = TestDeviceMetrics::new(vsock_metrics.clone());
        assert_eq!(metrics.vsock.activate_fails.count(), 1);
        metrics.vsock.activate_fails.inc();
        assert_eq!(vsock_metrics.activate_fails.count(), 2);
        assert_eq!(metrics.vsock.activate_fails.count(), 2);
    }
}
