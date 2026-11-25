-- Each price source's public face. Created empty: rows are added privately per
-- deployment, so display names and search pages never live in code.
-- `search_url_template` holds `{query}` where the medicine name goes.
CREATE TABLE price_sources (
    id                  text PRIMARY KEY
                        CHECK (id IN ('source_a', 'source_b', 'source_c', 'source_d')),
    display_name        text NOT NULL CHECK (length(display_name) BETWEEN 1 AND 40),
    search_url_template text NULL
                        CHECK (search_url_template IS NULL OR search_url_template LIKE 'https://%')
);

-- How many buyers `/v1/go` sent to each source per day. Counts only, nothing about who.
CREATE TABLE buy_clicks (
    day       date NOT NULL,
    source_id text NOT NULL,
    clicks    bigint NOT NULL DEFAULT 0,
    PRIMARY KEY (day, source_id)
);
