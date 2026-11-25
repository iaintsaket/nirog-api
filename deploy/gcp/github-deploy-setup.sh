#!/usr/bin/env bash
# One-time: lets GitHub Actions deploy this repo to Cloud Run with no stored key.
# Workload Identity Federation: Google trusts GitHub's own short-lived token, but only for
# REPO's main branch, and lets it act as a deploy service account with just the roles below.
# Prints the repo variables to set. Safe to re-run.
# Usage: PROJECT=nirog-prod REPO=iaintsaket/nirog-api deploy/gcp/github-deploy-setup.sh
set -euo pipefail
: "${PROJECT:?set PROJECT}" "${REPO:?set REPO (owner/name)}"
REGION="${REGION:-asia-southeast1}"
POOL=github
SA="github-deploy@$PROJECT.iam.gserviceaccount.com"
number=$(gcloud projects describe "$PROJECT" --format 'value(projectNumber)')
runtime_sa="$number-compute@developer.gserviceaccount.com" # runs the API and the builds

gcloud services enable --project "$PROJECT" iamcredentials.googleapis.com sts.googleapis.com
gcloud iam workload-identity-pools describe "$POOL" --project "$PROJECT" --location global >/dev/null 2>&1 \
  || gcloud iam workload-identity-pools create "$POOL" --project "$PROJECT" --location global --display-name "GitHub Actions"
gcloud iam workload-identity-pools providers describe "$POOL" --project "$PROJECT" --location global \
  --workload-identity-pool "$POOL" >/dev/null 2>&1 \
  || gcloud iam workload-identity-pools providers create-oidc "$POOL" --project "$PROJECT" --location global \
    --workload-identity-pool "$POOL" --display-name "GitHub" --issuer-uri "https://token.actions.githubusercontent.com" \
    --attribute-mapping "google.subject=assertion.sub,attribute.repository=assertion.repository,attribute.ref=assertion.ref" \
    --attribute-condition "assertion.repository == '$REPO' && assertion.ref == 'refs/heads/main'"

gcloud iam service-accounts describe "$SA" --project "$PROJECT" >/dev/null 2>&1 \
  || gcloud iam service-accounts create github-deploy --project "$PROJECT" --display-name "Deploys nirog-api from GitHub"
for _ in 1 2 3 4 5; do gcloud iam service-accounts describe "$SA" --project "$PROJECT" >/dev/null 2>&1 && break; sleep 5; done

# Only what a deploy and a rollback need.
for role in roles/run.admin roles/cloudbuild.builds.editor roles/artifactregistry.repoAdmin \
            roles/serviceusage.serviceUsageConsumer; do
  gcloud projects add-iam-policy-binding "$PROJECT" --member "serviceAccount:$SA" --role "$role" --condition None >/dev/null
done
# A dedicated bucket for build sources (uploads deleted after 7 days); the deploy account is admin
# of this bucket only, and the build reads from it as the runtime account.
bucket="gs://$PROJECT-build-src"
gcloud storage buckets describe "$bucket" >/dev/null 2>&1 \
  || gcloud storage buckets create "$bucket" --project "$PROJECT" --location "$REGION" --uniform-bucket-level-access
printf '{"rule":[{"action":{"type":"Delete"},"condition":{"age":7}}]}' > /tmp/build-src-lifecycle.json
gcloud storage buckets update "$bucket" --lifecycle-file /tmp/build-src-lifecycle.json >/dev/null
gcloud storage buckets add-iam-policy-binding "$bucket" --member "serviceAccount:$SA" --role roles/storage.admin >/dev/null
gcloud storage buckets add-iam-policy-binding "$bucket" --member "serviceAccount:$runtime_sa" \
  --role roles/storage.objectViewer >/dev/null
# Image repository with its cleanup policy (keep the 5 newest).
gcloud artifacts repositories describe nirog --project "$PROJECT" --location "$REGION" >/dev/null 2>&1 \
  || gcloud artifacts repositories create nirog --project "$PROJECT" --location "$REGION" --repository-format docker
gcloud artifacts repositories set-cleanup-policies nirog --project "$PROJECT" --location "$REGION" \
  --policy "$(dirname "$0")/cleanup-policy.json" --no-dry-run >/dev/null
# Deploy revisions and start builds that run as the runtime service account.
gcloud iam service-accounts add-iam-policy-binding "$runtime_sa" --project "$PROJECT" \
  --member "serviceAccount:$SA" --role roles/iam.serviceAccountUser >/dev/null
# GitHub runs of REPO's main branch may act as the deploy account.
gcloud iam service-accounts add-iam-policy-binding "$SA" --project "$PROJECT" --role roles/iam.workloadIdentityUser \
  --member "principalSet://iam.googleapis.com/projects/$number/locations/global/workloadIdentityPools/$POOL/attribute.repository/$REPO" >/dev/null

echo "Set these repo variables in $REPO (Settings → Secrets and variables → Actions → Variables):"
echo "GCP_PROJECT=$PROJECT"
echo "GCP_WIF_PROVIDER=projects/$number/locations/global/workloadIdentityPools/$POOL/providers/$POOL"
echo "GCP_DEPLOY_SA=$SA"
