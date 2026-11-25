-- The last few medicines each signed-in user opened, newest first.
-- Only what was opened is stored (match key, offer, label), never what was typed.
CREATE TABLE recent_medicines (
    user_id    uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    match_key  text NOT NULL,
    offer_id   text,
    label      text NOT NULL,
    opened_at  timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (user_id, match_key)
);
