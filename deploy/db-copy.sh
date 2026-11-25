#!/usr/bin/env bash
# Copy a whole Nirog database from one Postgres to another (local to Neon, Neon to RDS, ...).
# The target should be empty. Needs pg_dump and pg_restore at least as new as both servers.
# Usage: deploy/db-copy.sh "$SOURCE_URL" "$TARGET_URL"
set -euo pipefail
if [ $# -ne 2 ]; then
  echo "usage: $0 SOURCE_URL TARGET_URL" >&2
  exit 2
fi
pg_dump --format=custom --no-owner --no-acl "$1" \
  | pg_restore --no-owner --no-acl --exit-on-error --dbname="$2"
psql "$2" -Atc "select 'offers: ' || count(*) from offers"
