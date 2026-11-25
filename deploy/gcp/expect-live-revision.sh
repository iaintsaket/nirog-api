#!/usr/bin/env bash
# Checks the API serves all traffic from the expected release's revision and answers its health check.
# Usage: PROJECT=nirog-prod deploy/gcp/expect-live-revision.sh v1.0.1
set -euo pipefail
version="${1:?usage: $0 VERSION}"
: "${PROJECT:?set PROJECT}"
REGION="${REGION:-asia-southeast1}"
SERVICE="${SERVICE:-nirog-api}"
want="$SERVICE-$(tr . - <<< "$version")"
got=$(gcloud run services describe "$SERVICE" --project "$PROJECT" --region "$REGION" \
  --format 'value(status.traffic[0].revisionName,status.traffic[0].percent)')
url=$(gcloud run services describe "$SERVICE" --project "$PROJECT" --region "$REGION" --format 'value(status.url)')
code=$(curl -s -o /dev/null -w '%{http_code}' "$url/v1/health")
echo "Serving: $got; health: $code"
[ "$got" = "$want	100" ] && [ "$code" = 200 ]
