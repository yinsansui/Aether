#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StoredSystemConfigEntry {
    pub key: String,
    pub value: serde_json::Value,
    pub description: Option<String>,
    pub updated_at_unix_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AdminSecurityBlacklistEntry {
    pub ip_address: String,
    pub reason: String,
    pub ttl_seconds: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub struct AdminSystemStats {
    pub total_users: u64,
    pub active_users: u64,
    pub total_api_keys: u64,
    pub total_requests: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminSystemUsageAggregateImportMode {
    Skip,
    Overwrite,
    Error,
    ValidateError,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AdminSystemStatsDailyAggregate {
    pub date_unix_secs: u64,
    pub total_requests: u64,
    pub success_requests: u64,
    pub error_requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_cost: f64,
    pub actual_total_cost: f64,
    pub is_complete: bool,
    pub aggregated_at_unix_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AdminSystemStatsUserDailyAggregate {
    pub user_id: String,
    pub username: Option<String>,
    pub date_unix_secs: u64,
    pub total_requests: u64,
    pub success_requests: u64,
    pub error_requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_cost: f64,
    /// Billed amount for the day (what the user was actually charged).
    ///
    /// Snapshots written before `actual_total_cost` existed in this payload omit the field,
    /// which deserializes to `None`. Import treats `None` as "unknown" and keeps whatever the
    /// target database already has instead of overwriting it with 0.
    #[serde(default)]
    pub actual_total_cost: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AdminSystemStatsDailyApiKeyAggregate {
    pub api_key_id: String,
    pub api_key_name: Option<String>,
    pub date_unix_secs: u64,
    pub total_requests: u64,
    pub success_requests: u64,
    pub error_requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_cost: f64,
    /// Billed amount for the day (what the user was actually charged).
    ///
    /// See `AdminSystemStatsUserDailyAggregate::actual_total_cost` for the `None` semantics.
    #[serde(default)]
    pub actual_total_cost: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AdminSystemUsageAggregateSnapshot {
    #[serde(default)]
    pub stats_daily: Vec<AdminSystemStatsDailyAggregate>,
    #[serde(default)]
    pub stats_user_daily: Vec<AdminSystemStatsUserDailyAggregate>,
    #[serde(default)]
    pub stats_daily_api_key: Vec<AdminSystemStatsDailyApiKeyAggregate>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AdminSystemUsageAggregateImportCounter {
    pub created: u64,
    pub updated: u64,
    pub skipped: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AdminSystemUsageAggregateImportSummary {
    pub stats_daily: AdminSystemUsageAggregateImportCounter,
    pub stats_user_daily: AdminSystemUsageAggregateImportCounter,
    pub stats_daily_api_key: AdminSystemUsageAggregateImportCounter,
    pub skipped_unmapped_user_daily: u64,
    pub skipped_unmapped_api_key_daily: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminSystemPurgeTarget {
    Config,
    Users,
    Usage,
    AuditLogs,
    RequestBodies,
    Stats,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct AdminSystemPurgeSummary {
    pub affected: std::collections::BTreeMap<String, u64>,
}

impl AdminSystemPurgeSummary {
    pub fn add(&mut self, key: impl Into<String>, count: u64) {
        *self.affected.entry(key.into()).or_insert(0) += count;
    }

    pub fn merge(&mut self, other: &Self) {
        for (key, count) in &other.affected {
            self.add(key.clone(), *count);
        }
    }

    pub fn total(&self) -> u64 {
        self.affected.values().copied().sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_stats_user_daily() -> serde_json::Value {
        serde_json::json!({
            "user_id": "user-1",
            "username": "legacy",
            "date_unix_secs": 86400,
            "total_requests": 3,
            "success_requests": 3,
            "error_requests": 0,
            "input_tokens": 120,
            "output_tokens": 30,
            "cache_creation_tokens": 0,
            "cache_read_tokens": 0,
            "total_cost": 1.25,
        })
    }

    fn legacy_stats_daily_api_key() -> serde_json::Value {
        serde_json::json!({
            "api_key_id": "key-1",
            "api_key_name": "legacy",
            "date_unix_secs": 86400,
            "total_requests": 3,
            "success_requests": 3,
            "error_requests": 0,
            "input_tokens": 120,
            "output_tokens": 30,
            "cache_creation_tokens": 0,
            "cache_read_tokens": 0,
            "total_cost": 1.25,
        })
    }

    #[test]
    fn usage_aggregate_snapshot_loads_payloads_written_before_actual_total_cost() {
        let snapshot: AdminSystemUsageAggregateSnapshot =
            serde_json::from_value(serde_json::json!({
                "stats_user_daily": [legacy_stats_user_daily()],
                "stats_daily_api_key": [legacy_stats_daily_api_key()],
            }))
            .expect(
                "snapshots written before actual_total_cost was exported must still deserialize",
            );

        assert_eq!(snapshot.stats_user_daily[0].actual_total_cost, None);
        assert_eq!(snapshot.stats_daily_api_key[0].actual_total_cost, None);
    }

    #[test]
    fn usage_aggregate_snapshot_round_trips_actual_total_cost() {
        let mut snapshot = AdminSystemUsageAggregateSnapshot::default();
        snapshot
            .stats_user_daily
            .push(AdminSystemStatsUserDailyAggregate {
                user_id: "user-1".to_string(),
                username: Some("current".to_string()),
                date_unix_secs: 86400,
                total_requests: 3,
                success_requests: 3,
                error_requests: 0,
                input_tokens: 120,
                output_tokens: 30,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
                total_cost: 1.25,
                actual_total_cost: Some(0.5),
            });
        snapshot
            .stats_daily_api_key
            .push(AdminSystemStatsDailyApiKeyAggregate {
                api_key_id: "key-1".to_string(),
                api_key_name: Some("current".to_string()),
                date_unix_secs: 86400,
                total_requests: 3,
                success_requests: 3,
                error_requests: 0,
                input_tokens: 120,
                output_tokens: 30,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
                total_cost: 1.25,
                actual_total_cost: Some(0.5),
            });

        let encoded = serde_json::to_value(&snapshot).expect("snapshot should serialize");
        let decoded: AdminSystemUsageAggregateSnapshot = serde_json::from_value(encoded)
            .expect("snapshot with actual_total_cost should round-trip");

        assert_eq!(decoded, snapshot);
        assert_eq!(decoded.stats_user_daily[0].actual_total_cost, Some(0.5));
        assert_eq!(decoded.stats_daily_api_key[0].actual_total_cost, Some(0.5));
    }
}
