-- Round 6: idempotency request fingerprint (same key + different params
-- must fail instead of replaying the wrong result) + missing indexes.

ALTER TABLE idempotency_keys
  ADD COLUMN IF NOT EXISTS req JSONB NOT NULL DEFAULT '{}';

-- Revive-by-taker lookup.
CREATE INDEX IF NOT EXISTS idx_matches_taker
  ON matches (taker_id, created_at DESC, id DESC);

-- Future outbox publisher scan.
CREATE INDEX IF NOT EXISTS idx_outbox_unprocessed
  ON outbox_events (created_at) WHERE processed = FALSE;
