-- Audit fixes: pool/cap indexes + one-report-per-user guard.
-- matches FKs deferred to Phase 3 (given/taken semantics lock then).

CREATE INDEX IF NOT EXISTS idx_projects_pool
  ON projects (status, created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_projects_giver_day
  ON projects (giver_id, created_at DESC);

-- Same reporter cannot stack reports on one project (spec §6: 3 reports = hide).
CREATE UNIQUE INDEX IF NOT EXISTS idx_reports_once
  ON reports (project_id, reporter_id);
