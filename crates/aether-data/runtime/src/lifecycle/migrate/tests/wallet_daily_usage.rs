use sqlx::{query_as, raw_sql};

use super::{
    postgres_backend, prepare_and_apply_clean_postgres_database, ManagedPostgresServer, PgPool,
};
use crate::WalletDailyUsageAggregationInput;

#[tokio::test]
async fn wallet_daily_usage_aggregates_canonical_facts_and_replaces_only_the_target_day() {
    let Some(server) = ManagedPostgresServer::try_start()
        .await
        .expect("wallet ledger test database should start or skip")
    else {
        return;
    };
    let pool = PgPool::connect(server.database_url()).await.unwrap();
    prepare_and_apply_clean_postgres_database(&pool).await;

    raw_sql(
        r#"
INSERT INTO wallets (id, created_at, updated_at) VALUES
    ('current-wallet', now(), now()),
    ('free-wallet', now(), now()),
    ('stale-wallet', now(), now());

INSERT INTO usage (
    id, request_id, provider_name, model, billing_status, finalized_at, wallet_id,
    total_cost_usd, actual_total_cost_usd, input_tokens, output_tokens,
    cache_creation_input_tokens, cache_read_input_tokens
) VALUES
    ('snapshot', 'snapshot', 'provider', 'model', 'settled', '2026-11-22 23:59:59+08', NULL, 8, 6, 1, 2, 3, 4),
    ('legacy', 'legacy', 'provider', 'model', 'pending', '2026-11-23 23:59:59+08', NULL, 4, 1.75, 21, 22, 23, 24),
    ('free', 'free', 'provider', 'model', 'settled', '2026-11-23 04:00:00+08', NULL, 1, 2, 5, 6, 7, 8),
    ('no-price', 'no-price', 'provider', 'model', 'settled', '2026-11-23 04:00:00+08', NULL, 1, 1, 0, 0, 0, 0),
    ('unsettled', 'unsettled', 'provider', 'model', 'settled', '2026-11-23 04:00:00+08', NULL, 1, 1, 0, 0, 0, 0),
    ('next-day', 'next-day', 'provider', 'model', 'settled', '2026-11-23 04:00:00+08', NULL, 1, 1, 0, 0, 0, 0),
    ('no-snapshot', 'no-snapshot', 'provider', 'model', 'settled', '2026-11-23 04:00:00+08', 'stale-wallet', 1, 1, 0, 0, 0, 0);

INSERT INTO usage_settlement_snapshots (
    request_id, billing_status, wallet_id, finalized_at, billing_total_cost_usd,
    billing_actual_total_cost_usd, billing_input_tokens, billing_output_tokens,
    billing_cache_creation_5m_tokens, billing_cache_creation_1h_tokens, billing_cache_read_tokens
) VALUES
    ('snapshot', 'settled', 'current-wallet', '2026-11-23 00:00:00+08', 5, 2.5, 11, 12, 7, 9, 14),
    ('legacy', 'settled', 'current-wallet', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL),
    ('free', 'settled', 'free-wallet', '2026-11-23 04:00:00+08', 1, 0, NULL, NULL, NULL, NULL, NULL),
    ('no-price', 'settled', 'stale-wallet', '2026-11-23 04:00:00+08', 0, 3, NULL, NULL, NULL, NULL, NULL),
    ('unsettled', 'pending', 'stale-wallet', '2026-11-23 04:00:00+08', 1, 1, NULL, NULL, NULL, NULL, NULL),
    ('next-day', 'settled', 'stale-wallet', '2026-11-24 00:00:00+08', 1, 1, NULL, NULL, NULL, NULL, NULL);

INSERT INTO wallet_daily_usage_ledgers (
    id, wallet_id, billing_date, billing_timezone, total_requests, aggregated_at, created_at, updated_at
) VALUES
    ('old-free', 'free-wallet', '2026-11-23', 'Asia/Shanghai', 3, now(), now(), now()),
    ('old-stale', 'stale-wallet', '2026-11-23', 'Asia/Shanghai', 3, now(), now(), now()),
    ('other-tz', 'stale-wallet', '2026-11-23', 'UTC', 3, now(), now(), now()),
    ('other-day', 'stale-wallet', '2026-11-22', 'Asia/Shanghai', 3, now(), now(), now());
"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    let timestamp = |value| {
        chrono::DateTime::parse_from_rfc3339(value)
            .unwrap()
            .timestamp() as u64
    };
    let input = WalletDailyUsageAggregationInput {
        billing_date: "2026-11-23".to_string(),
        billing_timezone: "Asia/Shanghai".to_string(),
        window_start_unix_secs: timestamp("2026-11-23T00:00:00+08:00"),
        window_end_unix_secs: timestamp("2026-11-24T00:00:00+08:00"),
        aggregated_at_unix_secs: timestamp("2026-11-24T04:30:00+08:00"),
    };
    let backend = postgres_backend(server.database_url());
    let first_run = backend.aggregate_wallet_daily_usage(&input).await.unwrap();
    assert_eq!(first_run.aggregated_wallets, 2);
    assert_eq!(first_run.deleted_stale_ledgers, 1);

    let rerun = backend.aggregate_wallet_daily_usage(&input).await.unwrap();
    assert_eq!(rerun.aggregated_wallets, 2);
    assert_eq!(rerun.deleted_stale_ledgers, 0);

    let current: (f64, i32, i64, i64, i64, i64, bool) = query_as(
        r#"
SELECT total_cost_usd::double precision, total_requests, input_tokens, output_tokens,
       cache_creation_tokens, cache_read_tokens,
       first_finalized_at = '2026-11-23 00:00:00+08' AND last_finalized_at = '2026-11-23 23:59:59+08'
FROM wallet_daily_usage_ledgers WHERE wallet_id = 'current-wallet'
"#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(current, (4.25, 2, 32, 34, 39, 38, true));

    let other_ledgers: Vec<(String, f64, i32)> = query_as(
        "SELECT id, total_cost_usd::double precision, total_requests \
         FROM wallet_daily_usage_ledgers WHERE wallet_id <> 'current-wallet' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        other_ledgers,
        vec![
            ("old-free".to_string(), 0.0, 1),
            ("other-day".to_string(), 0.0, 3),
            ("other-tz".to_string(), 0.0, 3),
        ]
    );
}
