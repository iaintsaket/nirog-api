#!/usr/bin/env bash
# Cost guardrail for a Google Cloud project: a monthly budget that emails at 20%, 50% and 90%,
# and at 100% triggers a function that unlinks billing (paid services stop; nothing more is charged).
# Google reports cost a few hours late, so spend can slightly overshoot before the cut-off fires.
# Needs: gcloud logged in as a Billing Account Administrator of BILLING_ACCOUNT.
# Usage: PROJECT=nirog-prod BILLING_ACCOUNT=XXXXXX-XXXXXX-XXXXXX deploy/gcp/budget-guard.sh
set -euo pipefail
: "${PROJECT:?set PROJECT}" "${BILLING_ACCOUNT:?set BILLING_ACCOUNT}"
REGION="${REGION:-asia-southeast1}"
CAP="${CAP:-50}"            # in the billing account's currency
TOPIC="${TOPIC:-budget-guard}"
SA="budget-guard@$PROJECT.iam.gserviceaccount.com"
cd "$(dirname "$0")"

gcloud services enable --project "$PROJECT" pubsub.googleapis.com cloudfunctions.googleapis.com \
  eventarc.googleapis.com cloudbilling.googleapis.com billingbudgets.googleapis.com run.googleapis.com \
  cloudbuild.googleapis.com artifactregistry.googleapis.com
gcloud pubsub topics describe "$TOPIC" --project "$PROJECT" >/dev/null 2>&1 \
  || gcloud pubsub topics create "$TOPIC" --project "$PROJECT"

# Least privilege: Project Billing Manager on this project only can unlink its billing.
gcloud iam service-accounts describe "$SA" --project "$PROJECT" >/dev/null 2>&1 \
  || gcloud iam service-accounts create budget-guard --project "$PROJECT" --display-name "Unlinks billing at the budget cap"
# A new service account takes a few seconds to become visible to IAM, so retry.
for try in 1 2 3 4 5; do
  gcloud projects add-iam-policy-binding "$PROJECT" --member "serviceAccount:$SA" \
    --role roles/billing.projectManager --condition None >/dev/null && break
  [ "$try" = 5 ] && exit 1
  sleep 10
done

gcloud functions deploy budget-guard --gen2 --project "$PROJECT" --region "$REGION" \
  --runtime python313 --source budget-guard --entry-point stop_billing \
  --trigger-topic "$TOPIC" --service-account "$SA" --run-service-account "$SA" \
  --set-env-vars "PROJECT_ID=$PROJECT" --memory 256Mi --max-instances 1 --quiet
# The Pub/Sub trigger calls the function as $SA, which must be allowed to invoke it.
gcloud run services add-iam-policy-binding budget-guard --project "$PROJECT" --region "$REGION" \
  --member "serviceAccount:$SA" --role roles/run.invoker >/dev/null

budget=$(gcloud billing budgets list --billing-account "$BILLING_ACCOUNT" \
  --filter "displayName='$PROJECT cap'" --format 'value(name)' | head -1)
args=(--budget-amount "$CAP" --threshold-rule percent=0.2 --threshold-rule percent=0.5
  --threshold-rule percent=0.9 --threshold-rule percent=1.0
  --notifications-rule-pubsub-topic "projects/$PROJECT/topics/$TOPIC")
if [ -n "$budget" ]; then
  gcloud billing budgets update "$budget" --billing-account "$BILLING_ACCOUNT" --clear-threshold-rules "${args[@]}"
else
  gcloud billing budgets create --billing-account "$BILLING_ACCOUNT" --display-name "$PROJECT cap" \
    --filter-projects "projects/$PROJECT" "${args[@]}"
fi
echo "Guard ready: emails at 20/50/90% of $CAP, billing unlinked at 100%."
