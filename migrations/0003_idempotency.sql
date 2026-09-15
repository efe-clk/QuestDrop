-- Phase 3: idempotency keys for POST /v1/swaps.
-- Scoped per taker; a replay returns the stored response instead of
-- executing the swap twice.

CREATE TABLE IF NOT EXISTS idempotency_keys (
  key TEXT NOT NULL,
  taker_id UUID NOT NULL REFERENCES users(id),
  status_code INT NOT NULL,
  body JSONB NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (key, taker_id)
);
