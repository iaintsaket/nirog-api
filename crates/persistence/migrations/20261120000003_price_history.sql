-- Prices now accumulate per source instead of being replaced on each import.
-- `offers` keeps its shape and indexes, and is rebuilt from these tables after
-- every import as the newest known price of each product.

-- One dataset of prices, e.g. 'brands-2022' or 'jan-aushadhi'.
CREATE TABLE sources (
    id          text PRIMARY KEY,
    kind        text NOT NULL CHECK (kind IN ('jan_aushadhi', 'brand')),
    title       text NOT NULL,
    url         text,
    observed_on date NOT NULL,
    updated_at  timestamptz NOT NULL DEFAULT now()
);

-- One real product. `identity` (kind|name|maker|pack, normalised in Offer::identity)
-- lets a second source's listing of the same product attach to it.
CREATE TABLE products (
    id           text PRIMARY KEY,
    identity     text NOT NULL UNIQUE,
    kind         text NOT NULL CHECK (kind IN ('jan_aushadhi', 'brand')),
    name         text NOT NULL,
    manufacturer text NOT NULL,
    pack_count   integer NOT NULL CHECK (pack_count > 0),
    pack_unit    text NOT NULL CHECK (pack_unit IN ('unit', 'ml', 'g')),
    match_key    text NOT NULL,
    salts_label  text NOT NULL,
    first_seen   date NOT NULL,
    last_seen    date NOT NULL
);

-- A price a source saw on a date. Only ever added to (or corrected on re-import).
CREATE TABLE price_observations (
    product_id  text NOT NULL REFERENCES products (id),
    source_id   text NOT NULL REFERENCES sources (id),
    observed_on date NOT NULL,
    price_paise bigint NOT NULL CHECK (price_paise > 0),
    imported_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (product_id, source_id, observed_on)
);
CREATE INDEX price_observations_source ON price_observations (source_id);

CREATE TABLE import_runs (
    id          bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    source_id   text NOT NULL REFERENCES sources (id),
    observed_on date NOT NULL,
    offers      bigint NOT NULL,
    finished_at timestamptz NOT NULL DEFAULT now()
);

-- Backfill from the offers imported so far, so nothing is lost. The source ids
-- are the ones the importer uses, so the next import updates the same rows.
INSERT INTO sources (id, kind, title, url, observed_on)
SELECT 'jan-aushadhi', 'jan_aushadhi', 'Jan Aushadhi product list', NULL, min(imported_at)::date
FROM offers WHERE source = 'jan_aushadhi' HAVING count(*) > 0;
INSERT INTO sources (id, kind, title, url, observed_on)
SELECT 'brands-2022', 'brand', 'Indian Medicine Dataset (Nov 2022)',
       'https://github.com/junioralive/Indian-Medicine-Dataset', DATE '2022-11-17'
FROM offers WHERE source = 'brand' HAVING count(*) > 0;

CREATE TEMP TABLE legacy ON COMMIT DROP AS
SELECT o.*,
       o.source || '|' || btrim(regexp_replace(lower(o.name), '[^[:alnum:]]+', ' ', 'g'))
           || '|' || btrim(regexp_replace(lower(o.manufacturer), '[^[:alnum:]]+', ' ', 'g'))
           || '|' || o.pack_count || o.pack_unit AS identity,
       CASE o.source WHEN 'brand' THEN 'brands-2022' ELSE 'jan-aushadhi' END AS source_id
FROM offers o;

INSERT INTO products (id, identity, kind, name, manufacturer, pack_count, pack_unit,
                      match_key, salts_label, first_seen, last_seen)
SELECT DISTINCT ON (l.identity) l.id, l.identity, l.source, l.name, l.manufacturer,
       l.pack_count, l.pack_unit, l.match_key, l.salts_label, s.observed_on, s.observed_on
FROM legacy l JOIN sources s ON s.id = l.source_id
ORDER BY l.identity, l.id;

INSERT INTO price_observations (product_id, source_id, observed_on, price_paise)
SELECT p.id, l.source_id, s.observed_on, l.price_paise
FROM products p JOIN legacy l ON l.id = p.id JOIN sources s ON s.id = l.source_id;

-- Duplicate listings were folded into one product; drop their extra offers.
DELETE FROM offers o WHERE NOT EXISTS (SELECT 1 FROM products p WHERE p.id = o.id);
