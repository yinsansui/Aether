summary_totals AS (
  SELECT
    user_id,
    COALESCE(all_time_requests, 0)::BIGINT AS request_count,
    COALESCE(all_time_input_tokens + all_time_output_tokens
      + all_time_cache_creation_tokens + all_time_cache_read_tokens, 0)::BIGINT AS total_tokens
  FROM stats_user_summary
  WHERE user_id = ANY($1::TEXT[])
    AND (SELECT cutoff_date FROM daily_cutoff) IS NOT NULL
),
covered_hours AS (
  SELECT stats.hour_utc, stats.aggregated_at
  FROM stats_hourly AS stats
  WHERE stats.is_complete IS TRUE
    AND stats.hour_utc >= (SELECT cutoff_date FROM daily_cutoff)
    AND current_setting('aether.hourly_user_totals', true) IS DISTINCT FROM 'off'
    AND stats.user_totals_row_count IS NOT NULL
    -- A partial auxiliary import can replace some of this hour's local rows.
    AND stats.user_totals_row_count = (
      SELECT COUNT(*)
      FROM stats_hourly_user AS hourly
      WHERE hourly.hour_utc = stats.hour_utc
        AND hourly.updated_at = stats.aggregated_at
        AND hourly.total_tokens IS NOT NULL
    )
),
detail_ranges AS (
  SELECT
    COALESCE(LAG(hour_utc) OVER (ORDER BY hour_utc) + INTERVAL '1 hour',
      (SELECT cutoff_date FROM daily_cutoff)) AS range_start,
    hour_utc AS range_end
  FROM covered_hours
  UNION ALL
  SELECT
    COALESCE(MAX(hour_utc) + INTERVAL '1 hour', (SELECT cutoff_date FROM daily_cutoff), '-infinity'),
    'infinity'::timestamptz
  FROM covered_hours
),
totals AS (
  SELECT user_id, request_count, total_tokens
  FROM summary_totals
  UNION ALL
  SELECT
    user_id,
    COALESCE(SUM(total_requests), 0)::BIGINT,
    COALESCE(SUM(input_tokens + output_tokens + cache_creation_tokens + cache_read_tokens), 0)::BIGINT
  FROM stats_user_daily
  WHERE date < (SELECT cutoff_date FROM daily_cutoff)
    AND user_id = ANY(ARRAY(
      SELECT UNNEST($1::TEXT[])
      EXCEPT
      SELECT user_id::TEXT FROM summary_totals
    ))
  GROUP BY user_id
  UNION ALL
  SELECT hourly.user_id, hourly.total_requests, hourly.total_tokens
  FROM stats_hourly_user AS hourly
  JOIN covered_hours AS covered ON covered.hour_utc = hourly.hour_utc
    AND covered.aggregated_at = hourly.updated_at
  WHERE hourly.user_id = ANY($1::TEXT[])
    AND hourly.total_tokens IS NOT NULL
  UNION ALL
  SELECT detail.user_id, detail.request_count, detail.total_tokens
  FROM detail_ranges AS bounds
  CROSS JOIN LATERAL (
    SELECT
      "usage".user_id,
      COUNT(*)::BIGINT AS request_count,
      COALESCE(SUM(GREATEST(COALESCE("usage".total_tokens, 0), 0)), 0)::BIGINT AS total_tokens
    FROM usage_billing_facts AS "usage"
    WHERE "usage".created_at >= bounds.range_start
      AND "usage".created_at < bounds.range_end
      AND "usage".user_id = ANY($1::TEXT[])
      AND "usage".status NOT IN ('pending', 'streaming')
      AND "usage".provider_name NOT IN ('unknown', 'pending')
    GROUP BY "usage".user_id
  ) AS detail
  WHERE bounds.range_start < bounds.range_end
)
SELECT
  user_id,
  SUM(GREATEST(request_count, 0))::BIGINT AS request_count,
  SUM(GREATEST(total_tokens, 0))::BIGINT AS total_tokens
FROM totals
GROUP BY user_id
ORDER BY user_id ASC
