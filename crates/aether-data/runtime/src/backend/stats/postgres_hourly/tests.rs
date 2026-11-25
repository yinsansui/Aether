use std::time::Duration;

use aether_data_postgres::SqlxUsageReadRepository;
use chrono::{TimeZone, Utc};
use serde_json::Value;
use sqlx::{postgres::PgPoolOptions, Connection, PgConnection};

use super::{perform_stats_hourly_aggregation_for_hour, TRY_LOCK_STATS_HOURLY_USER_TOTALS_SQL};
use crate::lifecycle::export::{
    decode_jsonl, encode_jsonl, export_postgres_jsonl, import_postgres_jsonl, DataExportManifest,
    DataExportRecord, ExportDomain,
};
use crate::DatabaseDriver;

#[tokio::test]
#[ignore = "requires a disposable migrated PostgreSQL database in AETHER_TEST_POSTGRES_URL"]
async fn live_hourly_user_totals_match_the_detail_path() {
    let database_url = std::env::var("AETHER_TEST_POSTGRES_URL").unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .unwrap();
    let repository = SqlxUsageReadRepository::new(pool.clone());
    let user_ids = vec!["rollup-test-user".to_owned()];
    let hour = |hour| Utc.with_ymd_and_hms(2026, 11, 25, hour, 0, 0).unwrap();

    sqlx::raw_sql(
        r#"
INSERT INTO users (id, username, email, email_verified, password_hash, role, is_active, created_at, updated_at)
VALUES ('rollup-test-user', 'rollup-test-user', 'rollup@example.invalid', TRUE, 'x', 'user', TRUE, now(), now()),
       ('rollup-daily-user', 'rollup-daily-user', 'daily@example.invalid', TRUE, 'x', 'user', TRUE, now(), now());

INSERT INTO usage (id, request_id, user_id, provider_name, model, api_format, status, created_at,
                   input_tokens, output_tokens, total_tokens, cache_read_input_tokens)
SELECT request_id, request_id, 'rollup-test-user', provider_name, 'model', 'openai:responses', status,
       created_at::timestamptz, input_tokens, output_tokens, 0, cache_reads
FROM (VALUES
    ('rollup-legacy', 'provider', 'completed', '2026-11-25 00:30+00', 10, 1, 0),
    ('rollup-openai', 'provider', 'completed', '2026-11-25 01:00+00', 1000, 200, 300),
    ('rollup-pending', 'provider', 'pending', '2026-11-25 01:30+00', 10, 9, 0),
    ('rollup-excluded', 'unknown', 'completed', '2026-11-25 01:45+00', 900, 900, 0),
    ('rollup-gap', 'provider', 'completed', '2026-11-25 02:00+00', 10, 2, 0),
    ('rollup-late-settlement', 'provider', 'completed', '2026-11-25 03:00+00', 10, 3, 0),
    ('rollup-zero', 'provider', 'completed', '2026-11-25 03:30+00', 0, 0, 0),
    ('rollup-tail', 'provider', 'completed', '2026-11-25 04:00+00', 10, 4, 0)
) AS sample(request_id, provider_name, status, created_at, input_tokens, output_tokens, cache_reads);

INSERT INTO stats_summary (id, cutoff_date) VALUES ('rollup-summary', '2026-11-25 00:00+00');
INSERT INTO stats_user_summary (id, user_id, cutoff_date, all_time_requests,
                                all_time_input_tokens, all_time_output_tokens, all_time_cache_read_tokens)
VALUES ('rollup-user-summary', 'rollup-test-user', '2026-11-25 00:00+00', 2, 100, 20, 30);
INSERT INTO stats_user_daily (id, user_id, date, total_requests, input_tokens)
VALUES ('rollup-daily-fallback', 'rollup-daily-user', '2026-11-24 00:00+00', 1, 27);
"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    for bucket in [hour(0), hour(1), hour(3)] {
        perform_stats_hourly_aggregation_for_hour(&pool, bucket, hour(5))
            .await
            .unwrap();
    }
    sqlx::raw_sql(
        r#"
UPDATE stats_hourly SET user_totals_row_count = NULL WHERE hour_utc = '2026-11-25 00:00+00';
UPDATE stats_hourly_user SET total_tokens = NULL WHERE hour_utc = '2026-11-25 00:00+00';
"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    let hourly_tokens: i64 = sqlx::query_scalar(
        "SELECT total_tokens FROM stats_hourly_user WHERE hour_utc = $1 AND user_id = $2",
    )
    .bind(hour(1))
    .bind(&user_ids[0])
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(hourly_tokens, 1200);
    let totals = repository
        .summarize_usage_totals_by_user_ids(&user_ids)
        .await
        .unwrap();
    assert_eq!((totals[0].request_count, totals[0].total_tokens), (8, 1400));
    let daily_only = repository
        .summarize_usage_totals_by_user_ids(&["rollup-daily-user".to_owned()])
        .await
        .unwrap();
    assert_eq!(
        (daily_only[0].request_count, daily_only[0].total_tokens),
        (1, 27)
    );
    sqlx::query("SET aether.hourly_user_totals = 'off'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        repository
            .summarize_usage_totals_by_user_ids(&user_ids)
            .await
            .unwrap(),
        totals
    );
    sqlx::query("RESET aether.hourly_user_totals")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::raw_sql(
        r#"
UPDATE usage SET status = 'completed' WHERE request_id = 'rollup-pending';
INSERT INTO usage_settlement_snapshots (request_id, billing_status, billing_total_input_context, billing_output_tokens)
VALUES ('rollup-late-settlement', 'settled', 50, 10);
"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    let marked_hours: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM stats_hourly WHERE hour_utc IN ($1, $2) AND user_totals_row_count IS NOT NULL",
    )
    .bind(hour(1))
    .bind(hour(3))
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(marked_hours, 0);
    let revised = repository
        .summarize_usage_totals_by_user_ids(&user_ids)
        .await
        .unwrap();
    assert_eq!(
        (revised[0].request_count, revised[0].total_tokens),
        (9, 1466)
    );

    let mut writer = PgConnection::connect(&database_url).await.unwrap();
    let mut in_flight = writer.begin().await.unwrap();
    sqlx::query("UPDATE usage SET output_tokens = 9 WHERE request_id = 'rollup-tail'")
        .execute(&mut *in_flight)
        .await
        .unwrap();
    perform_stats_hourly_aggregation_for_hour(&pool, hour(4), hour(5))
        .await
        .unwrap();
    in_flight.commit().await.unwrap();

    let published: bool = sqlx::query_scalar(
        "SELECT user_totals_row_count IS NOT NULL FROM stats_hourly WHERE hour_utc = $1",
    )
    .bind(hour(4))
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!published);
    let revised = repository
        .summarize_usage_totals_by_user_ids(&user_ids)
        .await
        .unwrap();
    assert_eq!(
        (revised[0].request_count, revised[0].total_tokens),
        (9, 1471)
    );

    perform_stats_hourly_aggregation_for_hour(&pool, hour(1), hour(6))
        .await
        .unwrap();
    assert_hourly_import_uses_destination_facts(&pool, &repository, &user_ids, hour(1)).await;

    let mut historical_aggregation = pool.begin().await.unwrap();
    assert!(
        sqlx::query_scalar::<_, bool>(TRY_LOCK_STATS_HOURLY_USER_TOTALS_SQL)
            .bind(hour(4) - chrono::Duration::hours(32))
            .fetch_one(&mut *historical_aggregation)
            .await
            .unwrap()
    );
    tokio::time::timeout(
        Duration::from_secs(2),
        sqlx::query("UPDATE usage SET output_tokens = 10 WHERE request_id = 'rollup-tail'")
            .execute(&mut writer),
    )
    .await
    .expect("a write must not wait for a different hour's aggregation")
    .unwrap();
    historical_aggregation.rollback().await.unwrap();

    sqlx::raw_sql(
        r#"
UPDATE usage SET status = 'pending' WHERE request_id = 'rollup-pending';
INSERT INTO usage_settlement_snapshots (request_id, billing_status, billing_output_tokens)
VALUES ('rollup-openai', 'settled', 200);
"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    perform_stats_hourly_aggregation_for_hour(&pool, hour(1), hour(7))
        .await
        .unwrap();

    let mut purge = pool.begin().await.unwrap();
    sqlx::query("DELETE FROM usage_settlement_snapshots")
        .execute(&mut *purge)
        .await
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(2),
        sqlx::query("UPDATE usage SET status = 'completed' WHERE request_id = 'rollup-pending'")
            .execute(&mut writer),
    )
    .await
    .expect("purging snapshots must not lock the hourly row before deleting usage")
    .unwrap();
    sqlx::query("DELETE FROM usage WHERE user_id = $1")
        .bind(&user_ids[0])
        .execute(&mut *purge)
        .await
        .unwrap();
    purge.commit().await.unwrap();
    let purged = repository
        .summarize_usage_totals_by_user_ids(&user_ids)
        .await
        .unwrap();
    assert_eq!((purged[0].request_count, purged[0].total_tokens), (2, 150));
}

async fn assert_hourly_import_uses_destination_facts(
    pool: &sqlx::PgPool,
    repository: &SqlxUsageReadRepository,
    user_ids: &[String],
    bucket: chrono::DateTime<Utc>,
) {
    let export = export_postgres_jsonl(pool, vec![ExportDomain::Auxiliary], 0)
        .await
        .unwrap();
    let mut summary = None;
    let mut user = None;
    for record in decode_jsonl(&export).unwrap() {
        let DataExportRecord::Row { payload, .. } = &record else {
            continue;
        };
        if payload["hour_utc"]
            .as_str()
            .and_then(|value| value.parse().ok())
            != Some(bucket)
        {
            continue;
        }
        match payload["__table"].as_str() {
            Some("stats_hourly") => {
                assert!(payload.get("user_totals_row_count").is_none());
                summary = Some(record);
            }
            Some("stats_hourly_user") if payload["user_id"] == user_ids[0] => {
                assert!(payload.get("total_tokens").is_none());
                user = Some(record);
            }
            _ => {}
        }
    }
    let mut summary = summary.unwrap();
    let mut user = user.unwrap();
    if let DataExportRecord::Row { payload, .. } = &mut summary {
        payload["user_totals_row_count"] = Value::from(1);
    }
    if let DataExportRecord::Row { payload, .. } = &mut user {
        payload["total_requests"] = Value::from(1);
        payload["total_tokens"] = Value::from(30);
    }

    let manifest = DataExportRecord::manifest(DataExportManifest::new(
        0,
        Some(DatabaseDriver::Postgres),
        vec![ExportDomain::Auxiliary],
    ));
    let expected = repository
        .summarize_usage_totals_by_user_ids(user_ids)
        .await
        .unwrap();
    for (imported, expected_marker) in [
        (vec![manifest.clone(), user.clone()], Some(1)),
        (vec![manifest.clone(), summary.clone(), user], None),
        (vec![manifest, summary], None),
    ] {
        import_postgres_jsonl(pool, &encode_jsonl(&imported).unwrap())
            .await
            .unwrap();
        let marker: Option<i64> = sqlx::query_scalar(
            "SELECT user_totals_row_count FROM stats_hourly WHERE hour_utc = $1",
        )
        .bind(bucket)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(marker, expected_marker);
        let totals = repository
            .summarize_usage_totals_by_user_ids(user_ids)
            .await
            .unwrap();
        assert_eq!(totals, expected);
    }
}
