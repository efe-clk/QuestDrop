-- QuestDrop Phase 0/1 schema. Mirrors the frozen design doc.
-- pgvector embedding column reserved for phase C (extension not required in MVP).
-- NOTE: gen_random_uuid() is built into Postgres 13+, so no extension is needed.
-- (0001 checksum changed pre-production; no prod database exists yet.)

CREATE TABLE IF NOT EXISTS users (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  handle TEXT UNIQUE NOT NULL,
  email TEXT UNIQUE NOT NULL,
  can_do TEXT[] NOT NULL DEFAULT '{}',
  looking_for TEXT[] NOT NULL DEFAULT '{}',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

DO $$ BEGIN
  CREATE TYPE time_bucket AS ENUM ('S', 'M', 'L');
EXCEPTION WHEN duplicate_object THEN NULL; END $$;

DO $$ BEGIN
  CREATE TYPE energy AS ENUM ('LOW', 'MID', 'HIGH');
EXCEPTION WHEN duplicate_object THEN NULL; END $$;

DO $$ BEGIN
  CREATE TYPE project_status AS ENUM ('OPEN', 'MATCHED', 'CLOSED');
EXCEPTION WHEN duplicate_object THEN NULL; END $$;

DO $$ BEGIN
  CREATE TYPE offer_status AS ENUM ('OPEN', 'MATCHED', 'CLOSED');
EXCEPTION WHEN duplicate_object THEN NULL; END $$;

CREATE TABLE IF NOT EXISTS projects (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  title TEXT NOT NULL,
  one_liner TEXT NOT NULL,
  link_url TEXT NOT NULL,
  voice_url TEXT NOT NULL,
  skill_needed TEXT[] NOT NULL DEFAULT '{}',
  time_bucket time_bucket NOT NULL,
  energy energy NOT NULL,
  status project_status NOT NULL DEFAULT 'OPEN',
  embedding JSONB,
  giver_id UUID NOT NULL REFERENCES users(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS swap_offers (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id UUID NOT NULL REFERENCES projects(id),
  giver_id UUID NOT NULL REFERENCES users(id),
  status offer_status NOT NULL DEFAULT 'OPEN',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_swap_offers_status ON swap_offers(status);

CREATE TABLE IF NOT EXISTS matches (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  taker_id UUID NOT NULL,
  given_id UUID,
  taken_id UUID NOT NULL,
  score DOUBLE PRECISION NOT NULL,
  reason TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS outbox_events (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  type TEXT NOT NULL,
  payload JSONB NOT NULL,
  processed BOOLEAN NOT NULL DEFAULT FALSE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_outbox_processed ON outbox_events(processed);

CREATE TABLE IF NOT EXISTS reports (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id UUID NOT NULL REFERENCES projects(id),
  reporter_id UUID NOT NULL REFERENCES users(id),
  reason TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
