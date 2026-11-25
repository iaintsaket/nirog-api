-- Two more sources. Same ids as `Vendor::as_str`.
ALTER TABLE vendor_listings DROP CONSTRAINT vendor_listings_vendor_check;
ALTER TABLE vendor_listings ADD CONSTRAINT vendor_listings_vendor_check
    CHECK (vendor IN ('source_a', 'source_b', 'source_c', 'source_d'));
