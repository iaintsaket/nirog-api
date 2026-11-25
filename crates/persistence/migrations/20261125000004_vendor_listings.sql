-- What a price check last saw for a catalog offer at an online pharmacy.
-- offer_id has no foreign key, like saved_matches: imports rebuild `offers`,
-- and a listing must survive an offer briefly leaving and coming back.
CREATE TABLE vendor_listings (
    offer_id    text NOT NULL,
    vendor      text NOT NULL CHECK (vendor IN ('source_b', 'source_c')),
    url         text NOT NULL,
    status      text NOT NULL CHECK (status IN ('in_stock', 'out_of_stock', 'unknown')),
    price_paise bigint NOT NULL CHECK (price_paise > 0),
    checked_at  timestamptz NOT NULL,
    PRIMARY KEY (offer_id, vendor)
);
CREATE INDEX vendor_listings_vendor_checked ON vendor_listings (vendor, checked_at);
