-- 按“实际扣减金额”重新聚合 wallet_daily_usage_ledgers。
--
-- 背景：钱包“每日消费”历史上一度累加 usage.total_cost_usd（标准模型价，未乘 API Key 倍率），
-- 而钱包余额扣减走的是 usage.actual_total_cost_usd（已含 API Key 倍率，见
-- crates/aether-data/contracts/src/repository/settlement/types.rs 的 settlement_billable_cost_usd），
-- 于是“每日消费”与真实资金变动对不上。运行时聚合 SQL 已改为累加 actual，
-- 这里把历史账本行按同一口径重算一遍，否则只有新数据被修正。
--
-- 口径说明：
--   * 金额（total_cost_usd 列）改成 SUM(actual_total_cost_usd)。
--   * 计费请求判定仍按标准价 total_cost_usd > 0：free tier 的 actual 合法为 0，
--     若改成按 actual 过滤，这类请求会被从 total_requests 中剔除。
--   * 时间窗沿用账本行自带的 (billing_date, billing_timezone) 推导，与运行时一致；
--     不新增/重建不存在的日期行，缺失日期由日常维护任务补齐。
--
-- 可重复执行：多次运行结果一致（只依赖 usage_billing_facts / usage_settlement_snapshots 当前状态）。

CREATE TEMP TABLE tmp_wallet_daily_ledger_actual ON COMMIT DROP AS
SELECT
    ledgers.id AS ledger_id,
    COUNT(*)::BIGINT AS total_requests,
    CAST(COALESCE(SUM(usage.actual_total_cost_usd), 0) AS DOUBLE PRECISION) AS total_cost_usd,
    CAST(COALESCE(SUM(usage.input_tokens), 0) AS BIGINT) AS input_tokens,
    CAST(COALESCE(SUM(usage.output_tokens), 0) AS BIGINT) AS output_tokens,
    CAST(COALESCE(SUM(usage.cache_creation_input_tokens), 0) AS BIGINT) AS cache_creation_tokens,
    CAST(COALESCE(SUM(usage.cache_read_input_tokens), 0) AS BIGINT) AS cache_read_tokens,
    MIN(COALESCE(snapshots.finalized_at, usage.finalized_at)) AS first_finalized_at,
    MAX(COALESCE(snapshots.finalized_at, usage.finalized_at)) AS last_finalized_at
FROM wallet_daily_usage_ledgers AS ledgers
JOIN usage_settlement_snapshots AS snapshots
  ON snapshots.wallet_id = ledgers.wallet_id
JOIN usage_billing_facts AS usage
  ON usage.request_id = snapshots.request_id
 AND COALESCE(snapshots.finalized_at, usage.finalized_at)
       >= (ledgers.billing_date::timestamp AT TIME ZONE ledgers.billing_timezone)
 AND COALESCE(snapshots.finalized_at, usage.finalized_at)
       < ((ledgers.billing_date + 1)::timestamp AT TIME ZONE ledgers.billing_timezone)
WHERE COALESCE(snapshots.billing_status, usage.billing_status) = 'settled'
  AND usage.total_cost_usd > 0
GROUP BY ledgers.id;

UPDATE wallet_daily_usage_ledgers AS ledgers
SET
    total_cost_usd = rebuilt.total_cost_usd,
    total_requests = rebuilt.total_requests::integer,
    input_tokens = rebuilt.input_tokens,
    output_tokens = rebuilt.output_tokens,
    cache_creation_tokens = rebuilt.cache_creation_tokens,
    cache_read_tokens = rebuilt.cache_read_tokens,
    first_finalized_at = rebuilt.first_finalized_at,
    last_finalized_at = rebuilt.last_finalized_at,
    aggregated_at = NOW(),
    updated_at = NOW()
FROM tmp_wallet_daily_ledger_actual AS rebuilt
WHERE ledgers.id = rebuilt.ledger_id
  AND (
      ledgers.total_cost_usd IS DISTINCT FROM rebuilt.total_cost_usd
      OR ledgers.total_requests IS DISTINCT FROM rebuilt.total_requests::integer
      OR ledgers.input_tokens IS DISTINCT FROM rebuilt.input_tokens
      OR ledgers.output_tokens IS DISTINCT FROM rebuilt.output_tokens
      OR ledgers.cache_creation_tokens IS DISTINCT FROM rebuilt.cache_creation_tokens
      OR ledgers.cache_read_tokens IS DISTINCT FROM rebuilt.cache_read_tokens
      OR ledgers.first_finalized_at IS DISTINCT FROM rebuilt.first_finalized_at
      OR ledgers.last_finalized_at IS DISTINCT FROM rebuilt.last_finalized_at
  );

-- 与运行时维护任务保持一致：当天已没有任何“已结算”请求的账本行直接删掉。
DELETE FROM wallet_daily_usage_ledgers AS ledgers
WHERE NOT EXISTS (
    SELECT 1
    FROM tmp_wallet_daily_ledger_actual AS rebuilt
    WHERE rebuilt.ledger_id = ledgers.id
);
