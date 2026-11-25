SELECT COALESCE(
  (
    SELECT cutoff_date
    FROM stats_summary
    ORDER BY updated_at DESC, created_at DESC
    LIMIT 1
  ),
  (
    SELECT MAX(date) + INTERVAL '24 hours'
    FROM (
      SELECT MAX(date) AS date FROM stats_daily WHERE total_requests > 0 OR is_complete IS TRUE
      UNION ALL
      SELECT MAX(date) FROM stats_user_daily WHERE total_requests > 0
      UNION ALL
      SELECT MAX(date) FROM stats_daily_api_key WHERE total_requests > 0
    ) AS imported_daily_aggregates
  )
) AS cutoff_date
