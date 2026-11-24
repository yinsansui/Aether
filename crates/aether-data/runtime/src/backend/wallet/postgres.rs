use crate::backend::PostgresBackend;
use crate::error::SqlxResultExt;
use crate::{DataLayerError, WalletDailyUsageAggregationInput, WalletDailyUsageAggregationResult};

use super::unix_secs_to_utc;

const REBUILD_WALLET_DAILY_USAGE_LEDGERS_SQL: &str = r#"
WITH daily_requests AS (
    SELECT snapshot.request_id, snapshot.wallet_id, snapshot.finalized_at
    FROM usage_settlement_snapshots AS snapshot
    WHERE snapshot.billing_status = 'settled'
      AND snapshot.wallet_id IS NOT NULL
      AND snapshot.finalized_at >= $1
      AND snapshot.finalized_at < $2

    UNION ALL

    SELECT snapshot.request_id, snapshot.wallet_id, legacy.finalized_at
    FROM usage AS legacy
    JOIN usage_settlement_snapshots AS snapshot ON snapshot.request_id = legacy.request_id
    WHERE snapshot.billing_status = 'settled'
      AND snapshot.wallet_id IS NOT NULL
      AND snapshot.finalized_at IS NULL
      AND legacy.finalized_at >= $1
      AND legacy.finalized_at < $2
), aggregated AS MATERIALIZED (
    SELECT
        daily.wallet_id,
        COUNT(*) AS total_requests,
        -- wallet_daily_usage_ledgers.total_cost_usd 存的是用户实际被扣减的金额（已含 API Key 倍率），
        -- 即结算时真正从钱包余额扣除的 actual_total_cost_usd，而不是标准模型价。
        CAST(COALESCE(SUM(usage.actual_total_cost_usd), 0) AS DOUBLE PRECISION) AS total_cost_usd,
        COALESCE(SUM(usage.input_tokens), 0) AS input_tokens,
        COALESCE(SUM(usage.output_tokens), 0) AS output_tokens,
        COALESCE(SUM(usage.cache_creation_input_tokens), 0) AS cache_creation_tokens,
        COALESCE(SUM(usage.cache_read_input_tokens), 0) AS cache_read_tokens,
        MIN(daily.finalized_at) AS first_finalized_at,
        MAX(daily.finalized_at) AS last_finalized_at
    FROM daily_requests AS daily
    JOIN LATERAL (
        SELECT actual_total_cost_usd, input_tokens, output_tokens,
               cache_creation_input_tokens, cache_read_input_tokens
        FROM usage_billing_facts AS usage
        WHERE usage.request_id = daily.request_id
          -- Free-tier requests count when the list price is positive.
          AND usage.total_cost_usd > 0
        -- Keep the billing view lookup keyed by request_id after filtering the day.
        OFFSET 0
    ) AS usage ON TRUE
    GROUP BY daily.wallet_id
), upserted AS (
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
    RETURNING 1
), deleted AS (
    DELETE FROM wallet_daily_usage_ledgers AS ledgers
    WHERE ledgers.billing_date = $3
      AND ledgers.billing_timezone = $4
      -- Use the same snapshot and keep the delete disjoint from the upsert.
      AND NOT EXISTS (
          SELECT 1 FROM aggregated WHERE aggregated.wallet_id = ledgers.wallet_id
      )
    RETURNING 1
)
SELECT (SELECT COUNT(*) FROM upserted) AS aggregated_wallets,
       (SELECT COUNT(*) FROM deleted) AS deleted_stale_ledgers
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
        let (aggregated_wallets, deleted_stale_ledgers): (i64, i64) =
            sqlx::query_as(REBUILD_WALLET_DAILY_USAGE_LEDGERS_SQL)
                .bind(window_start)
                .bind(window_end)
                .bind(billing_date)
                .bind(input.billing_timezone.as_str())
                .bind(aggregated_at)
                .fetch_one(self.pool())
                .await
                .map_postgres_err()?;
        Ok(WalletDailyUsageAggregationResult {
            aggregated_wallets: usize::try_from(aggregated_wallets).unwrap_or(usize::MAX),
            deleted_stale_ledgers: usize::try_from(deleted_stale_ledgers).unwrap_or(usize::MAX),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::REBUILD_WALLET_DAILY_USAGE_LEDGERS_SQL;

    #[test]
    fn daily_ledger_amount_is_summed_from_actual_debited_cost() {
        let sql = REBUILD_WALLET_DAILY_USAGE_LEDGERS_SQL;

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
        let sql = REBUILD_WALLET_DAILY_USAGE_LEDGERS_SQL;

        assert!(sql.contains("usage.total_cost_usd > 0"));
        assert!(!sql.contains("usage.actual_total_cost_usd > 0"));
    }
}
