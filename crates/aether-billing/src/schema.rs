use std::collections::BTreeMap;

// 3.0 marks that `cost_breakdown` and `total_cost` already include the peak/off-peak multiplier,
// so consumers no longer read them as the bare catalog price.
pub const BILLING_SNAPSHOT_SCHEMA_VERSION: &str = "3.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingSnapshotStatus {
    Complete,
    Incomplete,
    NoRule,
    Legacy,
}

impl BillingSnapshotStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Incomplete => "incomplete",
            Self::NoRule => "no_rule",
            Self::Legacy => "legacy",
        }
    }
}

impl std::fmt::Display for BillingSnapshotStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BillingSnapshot {
    pub schema_version: String,
    pub rule_id: Option<String>,
    pub rule_name: Option<String>,
    pub scope: Option<String>,
    pub expression: Option<String>,
    pub resolved_dimensions: BTreeMap<String, serde_json::Value>,
    /// Formula inputs resolved from the selected catalog before time and API-key multipliers.
    pub resolved_variables: BTreeMap<String, serde_json::Value>,
    pub cost_breakdown: BTreeMap<String, f64>,
    pub total_cost: f64,
    pub tier_index: Option<i64>,
    pub tier_info: Option<serde_json::Value>,
    pub missing_required: Vec<String>,
    pub status: BillingSnapshotStatus,
    pub calculated_at: String,
    pub engine_version: String,
}

impl Default for BillingSnapshot {
    fn default() -> Self {
        Self {
            schema_version: BILLING_SNAPSHOT_SCHEMA_VERSION.to_string(),
            rule_id: None,
            rule_name: None,
            scope: None,
            expression: None,
            resolved_dimensions: BTreeMap::new(),
            resolved_variables: BTreeMap::new(),
            cost_breakdown: BTreeMap::new(),
            total_cost: 0.0,
            tier_index: None,
            tier_info: None,
            missing_required: Vec::new(),
            status: BillingSnapshotStatus::NoRule,
            calculated_at: String::new(),
            engine_version: "2.0".to_string(),
        }
    }
}

impl BillingSnapshot {
    pub fn dimensions_used(&self) -> &BTreeMap<String, serde_json::Value> {
        &self.resolved_dimensions
    }

    pub fn cost(&self) -> f64 {
        self.total_cost
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CostResult {
    pub cost: f64,
    pub status: BillingSnapshotStatus,
    pub snapshot: BillingSnapshot,
}

#[cfg(test)]
mod tests {
    use super::{BillingSnapshot, BillingSnapshotStatus};

    #[test]
    fn snapshot_aliases_dimensions_and_cost() {
        let mut snapshot = BillingSnapshot {
            total_cost: 1.25,
            status: BillingSnapshotStatus::Complete,
            ..BillingSnapshot::default()
        };
        snapshot
            .resolved_dimensions
            .insert("input_tokens".to_string(), serde_json::json!(10));

        assert_eq!(snapshot.cost(), 1.25);
        assert_eq!(
            snapshot.dimensions_used().get("input_tokens"),
            Some(&serde_json::json!(10))
        );
    }
}
