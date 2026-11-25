# PRD-4: Map of Jan Aushadhi Kendras near me

Covers request 4 (first half).

## Goal
A map and list of Jan Aushadhi Kendras sorted by distance from the user's location, reachable from every
Jan Aushadhi offer ("Nearest Kendra: 1.2 km") and from `/kendras`.

## Data
- PMBI publishes the Kendra list (code, name, address, state, district, pincode) on janaushadhi.gov.in.
  It has no coordinates. Geocode by pincode centroid from the government All-India Pincode directory
  (data.gov.in, has lat/long); mark these as "approximate (pincode area)". Imported by `apps/importer`
  as a new source; committed CSV with the date it was published, like the product list.
- Agent must first confirm both files are downloadable and their licence allows reuse; stop and report if not.

## Design
- Table `kendras(code, name, address, pincode, lat, lon, precision)`; `GET /v1/kendras/near?lat&lon&limit=20`
  (Haversine in SQL, bounding-box prefilter; no PostGIS needed for ~16k rows).
- Location: browser Geolocation, asked only on tap; fallback pincode box. Never sent except as the query, never logged/stored.
- Map: Leaflet + OpenStreetMap tiles (free, no key). Each pin: name, address, distance, "Directions" (Google Maps URL).
- Stock per Kendra is not public; say "call to confirm stock".

## Acceptance
- Distance ordering unit tested; query p95 < 100 ms on full data.
- Page works with location denied (pincode path). Layout checks pass on phone + desktop.
