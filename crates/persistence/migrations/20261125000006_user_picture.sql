-- The Google profile picture shown in the website header. Refreshed on every sign-in.
ALTER TABLE users ADD COLUMN picture_url text
    CHECK (picture_url IS NULL OR (picture_url LIKE 'https://%' AND length(picture_url) <= 500));
