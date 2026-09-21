use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct HousekeepingReport {
    pub through_seq: u64,
    pub pruned_events: u64,
    pub retention_epoch: u64,
}

pub(crate) const AUDIT_RETENTION_MS: i64 = 365 * 86_400_000;
pub(crate) const DETAIL_RETENTION_MS: i64 = 90 * 86_400_000;
pub(crate) const HOURLY_RETENTION_MS: i64 = 90 * 86_400_000;
pub(crate) const DAILY_RETENTION_MS: i64 = 365 * 86_400_000;
pub(crate) const RETENTION_DELETE_LIMIT: i64 = 500;
pub(crate) const HARD_EVENT_COUNT: i64 = 2_000_000;
pub(crate) const MAX_PAGE_COUNT: i64 = 131_072;
pub(crate) const TERMINAL_RESERVE_PAGES: i64 = 4_096;
