use crate::backend::PostgresBackend;
use crate::error::SqlxResultExt;
use crate::{DataLayerError, WalletDailyUsageAggregationInput, WalletDailyUsageAggregationResult};

use super::unix_secs_to_utc;

const UPSERT_WALLET_DAILY_USAGE_LEDGER_SQL: &str = r#"
WITH aggregated AS (
    SELECT
        usage_settlement_snapshots.wallet_id,
        COUNT(*) AS total_requests,
        -- wallet_daily_usage_ledgers.total_cost_usd 存的是用户实际被扣减的金额（已含 API Key 倍率），
        -- 即结算时真正从钱包余额扣除的 actual_total_cost_usd，而不是标准模型价。
        CAST(COALESCE(SUM(usage.actual_total_cost_usd), 0) AS DOUBLE PRECISION) AS total_cost_usd,
        COALESCE(SUM(usage.input_tokens), 0) AS input_tokens,
        COALESCE(SUM(usage.output_tokens), 0) AS output_tokens,
        COALESCE(SUM(usage.cache_creation_input_tokens), 0) AS cache_creation_tokens,
        COALESCE(SUM(usage.cache_read_input_tokens), 0) AS cache_read_tokens,
        MIN(COALESCE(usage_settlement_snapshots.finalized_at, usage.finalized_at)) AS first_finalized_at,
        MAX(COALESCE(usage_settlement_snapshots.finalized_at, usage.finalized_at)) AS last_finalized_at
    FROM usage_billing_facts AS usage
    JOIN usage_settlement_snapshots
      ON usage_settlement_snapshots.request_id = usage.request_id
    WHERE usage_settlement_snapshots.wallet_id IS NOT NULL
      AND COALESCE(usage_settlement_snapshots.billing_status, usage.billing_status) = 'settled'
      -- 计费请求判定继续按标准价：free tier 的 actual_total_cost_usd 合法为 0，
      -- 若改成 actual_total_cost_usd > 0 会把这类请求从 total_requests 里剔除。
      AND usage.total_cost_usd > 0
      AND COALESCE(usage_settlement_snapshots.finalized_at, usage.finalized_at) >= $1
      AND COALESCE(usage_settlement_snapshots.finalized_at, usage.finalized_at) < $2
    GROUP BY usage_settlement_snapshots.wallet_id
)
INSERT INTO wallet_daily_usage_ledgers (
    id,
    wallet_id,
    billing_date,
    billing_timezone,
    total_cost_usd,
    total_requests,
    input_tokens,
    output_tokens,
    cache_creation_tokens,
    cache_read_tokens,
    first_finalized_at,
    last_finalized_at,
    aggregated_at,
    created_at,
    updated_at
)
SELECT
    md5(CONCAT('wallet-daily-usage:', aggregated.wallet_id, ':', CAST($3 AS TEXT), ':', $4)),
    aggregated.wallet_id,
    $3,
    $4,
    aggregated.total_cost_usd,
    aggregated.total_requests,
    aggregated.input_tokens,
    aggregated.output_tokens,
    aggregated.cache_creation_tokens,
    aggregated.cache_read_tokens,
    aggregated.first_finalized_at,
    aggregated.last_finalized_at,
    $5,
    $5,
    $5
FROM aggregated
ON CONFLICT (wallet_id, billing_date, billing_timezone)
DO UPDATE SET
    total_cost_usd = EXCLUDED.total_cost_usd,
    total_requests = EXCLUDED.total_requests,
    input_tokens = EXCLUDED.input_tokens,
    output_tokens = EXCLUDED.output_tokens,
    cache_creation_tokens = EXCLUDED.cache_creation_tokens,
    cache_read_tokens = EXCLUDED.cache_read_tokens,
    first_finalized_at = EXCLUDED.first_finalized_at,
    last_finalized_at = EXCLUDED.last_finalized_at,
    aggregated_at = EXCLUDED.aggregated_at,
    updated_at = EXCLUDED.updated_at
"#;

const DELETE_STALE_WALLET_DAILY_USAGE_LEDGERS_SQL: &str = r#"
DELETE FROM wallet_daily_usage_ledgers AS ledgers
WHERE ledgers.billing_date = $1
  AND ledgers.billing_timezone = $2
  AND NOT EXISTS (
      SELECT 1
      FROM usage_billing_facts AS usage
      JOIN usage_settlement_snapshots
        ON usage_settlement_snapshots.request_id = usage.request_id
      WHERE usage_settlement_snapshots.wallet_id = ledgers.wallet_id
        AND COALESCE(usage_settlement_snapshots.billing_status, usage.billing_status) = 'settled'
        -- 与上面的日聚合保持同一批“计费请求”：按标准价判断，不能改成 actual，
        -- 否则 free tier（actual = 0）的请求会被判成不存在，导致整天的账本被删除。
        AND usage.total_cost_usd > 0
        AND COALESCE(usage_settlement_snapshots.finalized_at, usage.finalized_at) >= $3
        AND COALESCE(usage_settlement_snapshots.finalized_at, usage.finalized_at) < $4
  )
"#;

impl PostgresBackend {
    pub async fn aggregate_wallet_daily_usage(
        &self,
        input: &WalletDailyUsageAggregationInput,
    ) -> Result<WalletDailyUsageAggregationResult, DataLayerError> {
        let billing_date = chrono::NaiveDate::parse_from_str(&input.billing_date, "%Y-%m-%d")
            .map_err(|err| {
                DataLayerError::InvalidInput(format!("invalid wallet billing_date: {err}"))
            })?;
        let window_start = unix_secs_to_utc(input.window_start_unix_secs, "window_start")?;
        let window_end = unix_secs_to_utc(input.window_end_unix_secs, "window_end")?;
        let aggregated_at = unix_secs_to_utc(input.aggregated_at_unix_secs, "aggregated_at")?;
        let mut tx = self.pool().begin().await.map_postgres_err()?;

        let aggregated_wallets = sqlx::query(UPSERT_WALLET_DAILY_USAGE_LEDGER_SQL)
            .bind(window_start)
            .bind(window_end)
            .bind(billing_date)
            .bind(input.billing_timezone.as_str())
            .bind(aggregated_at)
            .execute(&mut *tx)
            .await
            .map_postgres_err()?
            .rows_affected();

        let deleted_stale_ledgers = sqlx::query(DELETE_STALE_WALLET_DAILY_USAGE_LEDGERS_SQL)
            .bind(billing_date)
            .bind(input.billing_timezone.as_str())
            .bind(window_start)
            .bind(window_end)
            .execute(&mut *tx)
            .await
            .map_postgres_err()?
            .rows_affected();

        tx.commit().await.map_postgres_err()?;
        Ok(WalletDailyUsageAggregationResult {
            aggregated_wallets: usize::try_from(aggregated_wallets).unwrap_or(usize::MAX),
            deleted_stale_ledgers: usize::try_from(deleted_stale_ledgers).unwrap_or(usize::MAX),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DELETE_STALE_WALLET_DAILY_USAGE_LEDGERS_SQL, UPSERT_WALLET_DAILY_USAGE_LEDGER_SQL,
    };

    #[test]
    fn daily_ledger_amount_is_summed_from_actual_debited_cost() {
        let sql = UPSERT_WALLET_DAILY_USAGE_LEDGER_SQL;

        assert!(
            sql.contains("CAST(COALESCE(SUM(usage.actual_total_cost_usd), 0) AS DOUBLE PRECISION) AS total_cost_usd"),
            "wallet daily ledger amount must aggregate the real debited cost"
        );
        assert!(
            !sql.contains("SUM(usage.total_cost_usd)"),
            "wallet daily ledger must not aggregate list-price total_cost_usd"
        );
    }

    #[test]
    fn daily_ledger_population_still_keys_off_list_price() {
        for (name, sql) in [
            ("upsert", UPSERT_WALLET_DAILY_USAGE_LEDGER_SQL),
            ("delete_stale", DELETE_STALE_WALLET_DAILY_USAGE_LEDGERS_SQL),
        ] {
            assert_eq!(
                sql.matches("usage.total_cost_usd > 0").count(),
                1,
                "{name} must keep filtering billable requests by list price"
            );
            assert!(
                !sql.contains("usage.actual_total_cost_usd > 0"),
                "{name} must not filter billable requests by actual cost: free tier rows \
                 (actual = 0) would be dropped from total_requests"
            );
        }
    }
}
