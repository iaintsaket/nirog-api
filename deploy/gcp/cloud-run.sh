#!/usr/bin/env bash
# Build the image with Cloud Build and deploy it to Cloud Run.
# Needs: gcloud logged in, a project with Cloud Run, Cloud Build, Artifact Registry and
# Secret Manager enabled, and these secrets created once in Secret Manager:
#   nirog-database-url, nirog-proxy-secret, nirog-google-client-secret, nirog-pages-token
# Usage: PROJECT=my-project WEB_ORIGIN=https://nirog.pages.dev GOOGLE_CLIENT_ID=... deploy/gcp/cloud-run.sh
set -euo pipefail
: "${PROJECT:?set PROJECT}" "${WEB_ORIGIN:?set WEB_ORIGIN}" "${GOOGLE_CLIENT_ID:?set GOOGLE_CLIENT_ID}"
REGION="${REGION:-asia-southeast1}"  # beside the Neon database in Singapore
SERVICE="${SERVICE:-nirog-api}"
REPO="${REPO:-nirog}"
cd "$(dirname "$0")/../.."
# VERSION (v1.0.1) names the image and the Cloud Run revision, so deploy/gcp/rollback.sh can find it.
VERSION="${VERSION:-}"
tag="${VERSION:-$(git rev-parse --short HEAD)}"
# IMAGE: an image someone already built and pushed (the GitHub deploy does); otherwise Cloud Build builds it.
prebuilt="${IMAGE:-}"
image="${IMAGE:-$REGION-docker.pkg.dev/$PROJECT/$REPO/$SERVICE:$tag}"
revision_args=()
[ -n "$VERSION" ] && revision_args=(--revision-suffix "$(tr . - <<< "$VERSION")")

# The image repository and its cleanup policy (keep the 5 newest images, enough to roll back to
# recent releases, inside the free storage) are set once, here or by github-deploy-setup.sh.
if ! gcloud artifacts repositories describe "$REPO" --project "$PROJECT" --location "$REGION" >/dev/null 2>&1; then
  gcloud artifacts repositories create "$REPO" --project "$PROJECT" --location "$REGION" --repository-format docker
  gcloud artifacts repositories set-cleanup-policies "$REPO" --project "$PROJECT" --location "$REGION" \
    --policy deploy/gcp/cleanup-policy.json --no-dry-run >/dev/null
fi
# Sources go to a dedicated bucket (STAGING_BUCKET) so the deploy account needs no project-wide
# storage access. The build starts without following its logs (that needs project Viewer), and
# its status is checked until it finishes. Logs: Cloud Build in the console.
staging="${STAGING_BUCKET:-gs://$PROJECT-build-src}"
if [ -z "$prebuilt" ]; then
build=$(gcloud builds submit --project "$PROJECT" --region "$REGION" --async --format 'value(id)' \
  --gcs-source-staging-dir "$staging/source" --config deploy/gcp/cloudbuild.yaml --substitutions "_IMAGE=$image" .)
echo "Building $image (Cloud Build $build)"
while :; do
  status=$(gcloud builds describe "$build" --project "$PROJECT" --region "$REGION" --format 'value(status)')
  case "$status" in
    SUCCESS) break ;;
    QUEUED|WORKING|PENDING) sleep 15 ;;
    *) echo "Build $build ended with $status: https://console.cloud.google.com/cloud-build/builds;region=$REGION/$build?project=$PROJECT" >&2; exit 1 ;;
  esac
done
fi
gcloud run deploy "$SERVICE" --project "$PROJECT" --region "$REGION" --image "$image" "${revision_args[@]}" \
  --allow-unauthenticated --min-instances 0 --max-instances 1 --memory 512Mi --cpu 1 --cpu-throttling --concurrency 80 --timeout 30 \
  --set-env-vars "WEB_ORIGIN=$WEB_ORIGIN,GOOGLE_CLIENT_ID=$GOOGLE_CLIENT_ID" \
  --set-secrets "DATABASE_URL=nirog-database-url:latest,PROXY_SECRET=nirog-proxy-secret:latest,GOOGLE_CLIENT_SECRET=nirog-google-client-secret:latest,PAGES_TOKEN=nirog-pages-token:latest"
# After a rollback, traffic is pinned to an old revision; a new release takes all traffic again.
gcloud run services update-traffic "$SERVICE" --project "$PROJECT" --region "$REGION" --to-latest >/dev/null
