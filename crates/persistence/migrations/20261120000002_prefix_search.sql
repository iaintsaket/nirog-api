-- Two-letter searches match name and salt prefixes; these indexes serve LIKE 'xy%'.
CREATE INDEX offers_name_prefix ON offers (lower(name) text_pattern_ops);
CREATE INDEX offers_salts_prefix ON offers (lower(salts_label) text_pattern_ops);
