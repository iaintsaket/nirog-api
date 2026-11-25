#!/usr/bin/env bash
# Instantly send all API traffic to an earlier release's Cloud Run revision. No rebuild: Cloud Run
# keeps old revisions, and Artifact Registry keeps the 5 newest images they need to start.
# Usage: PROJECT=nirog-prod deploy/gcp/rollback.sh v1.0.0
set -euo pipefail
version="${1:?usage: $0 VERSION}"
: "${PROJECT:?set PROJECT}"
REGION="${REGION:-asia-southeast1}"
SERVICE="${SERVICE:-nirog-api}"
revision="$SERVICE-$(tr . - <<< "$version")"
if ! gcloud run revisions describe "$revision" --project "$PROJECT" --region "$REGION" >/dev/null 2>&1; then
  echo "No revision $revision. Releases: https://github.com/iaintsaket/nirog-api/releases" >&2
  exit 1
fi
gcloud run services update-traffic "$SERVICE" --project "$PROJECT" --region "$REGION" --to-revisions "$revision=100" >/dev/null
echo "API now serves $version ($revision)"
