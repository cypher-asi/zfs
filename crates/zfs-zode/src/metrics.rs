use std::sync::atomic::{AtomicU64, Ordering};

/// Atomic counters and gauges exposed for UI and monitoring.
#[derive(Debug, Default)]
pub struct ZodeMetrics {
    /// Total sector entries successfully stored.
    pub sectors_stored_total: AtomicU64,
    /// Total store requests rejected (all reasons).
    pub store_rejections_total: AtomicU64,
    /// Rejections due to policy (program not subscribed).
    pub policy_rejections: AtomicU64,
    /// Rejections due to storage limits exceeded.
    pub limit_rejections: AtomicU64,
    /// Current connected peer count.
    pub peer_count: AtomicU64,
    /// Current approximate DB size in bytes.
    pub db_size_bytes: AtomicU64,
}

impl ZodeMetrics {
    pub fn inc_sectors_stored(&self) {
        self.sectors_stored_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_policy_rejection(&self) {
        self.store_rejections_total.fetch_add(1, Ordering::Relaxed);
        self.policy_rejections.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_limit_rejection(&self) {
        self.store_rejections_total.fetch_add(1, Ordering::Relaxed);
        self.limit_rejections.fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_peer_count(&self, count: u64) {
        self.peer_count.store(count, Ordering::Relaxed);
    }

    pub fn inc_peer_count(&self) {
        self.peer_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn dec_peer_count(&self) {
        self.peer_count.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn set_db_size(&self, size: u64) {
        self.db_size_bytes.store(size, Ordering::Relaxed);
    }

    /// Snapshot all counters for display.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            sectors_stored_total: self.sectors_stored_total.load(Ordering::Relaxed),
            store_rejections_total: self.store_rejections_total.load(Ordering::Relaxed),
            policy_rejections: self.policy_rejections.load(Ordering::Relaxed),
            limit_rejections: self.limit_rejections.load(Ordering::Relaxed),
            peer_count: self.peer_count.load(Ordering::Relaxed),
            db_size_bytes: self.db_size_bytes.load(Ordering::Relaxed),
        }
    }
}

/// Point-in-time snapshot of Zode metrics (non-atomic, safe to display/serialize).
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct MetricsSnapshot {
    pub sectors_stored_total: u64,
    pub store_rejections_total: u64,
    pub policy_rejections: u64,
    pub limit_rejections: u64,
    pub peer_count: u64,
    pub db_size_bytes: u64,
}
