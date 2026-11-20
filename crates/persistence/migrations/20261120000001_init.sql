-- Project Nirog initial schema.
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- One buyable product from one source. Replaced wholesale per source on import.
CREATE TABLE offers (
    id                    text PRIMARY KEY,
    source                text NOT NULL CHECK (source IN ('jan_aushadhi', 'brand')),
    name                  text NOT NULL,
    manufacturer          text NOT NULL,
    price_paise           bigint NOT NULL CHECK (price_paise > 0),
    pack_count            integer NOT NULL CHECK (pack_count > 0),
    pack_unit             text NOT NULL CHECK (pack_unit IN ('unit', 'ml', 'g')),
    unit_price_millipaise bigint NOT NULL,
    match_key             text NOT NULL,
    salts_label           text NOT NULL,
    -- Source-specific fields that do not drive matching (room for AMRIT, NPPA, ...).
    extra                 jsonb NOT NULL DEFAULT '{}'::jsonb,
    imported_at           timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX offers_match_key_price ON offers (match_key, unit_price_millipaise);
CREATE INDEX offers_source ON offers (source);
CREATE INDEX offers_name_trgm ON offers USING gin (lower(name) gin_trgm_ops);
CREATE INDEX offers_salts_trgm ON offers USING gin (lower(salts_label) gin_trgm_ops);

CREATE TABLE users (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    provider   text NOT NULL,
    subject    text NOT NULL,
    email      text NOT NULL,
    name       text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (provider, subject)
);

-- Only the SHA-256 of the session token is stored; the raw token lives in the cookie.
CREATE TABLE sessions (
    token_hash bytea PRIMARY KEY CHECK (length(token_hash) = 32),
    user_id    uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    expires_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX sessions_user ON sessions (user_id);
CREATE INDEX sessions_expires ON sessions (expires_at);

-- OAuth state (hashed) -> PKCE verifier. Rows are deleted when used.
CREATE TABLE login_states (
    state_hash    bytea PRIMARY KEY CHECK (length(state_hash) = 32),
    pkce_verifier text NOT NULL,
    expires_at    timestamptz NOT NULL
);
CREATE INDEX login_states_expires ON login_states (expires_at);

CREATE TABLE saved_matches (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    match_key  text NOT NULL,
    offer_id   text,
    label      text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (user_id, match_key)
);
