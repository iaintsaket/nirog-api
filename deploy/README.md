# Deploying nirog-api

The API is one container image (`Dockerfile`) that reads every setting from environment
variables and applies its own database migrations on start. Any host that runs a container and
any Postgres 15+ with the `pg_trgm` extension will do. Nothing in the code depends on a provider.

## Settings

| Variable | Required | Meaning |
|---|---|---|
| `DATABASE_URL` | yes (secret) | Postgres connection string, with `sslmode=require` for hosted databases. |
| `WEB_ORIGIN` | yes | The website's address, for example `https://nirog.pages.dev`. Used for CORS and redirects. |
| `PROXY_SECRET` | yes in production (secret) | At least 32 random characters. The web proxy sends it; other callers are refused. |
| `GOOGLE_CLIENT_ID` | yes | Google sign-in client. Any placeholder works while sign-in is hidden. |
| `GOOGLE_CLIENT_SECRET` | yes (secret) | As above. |
| `PAGES_TOKEN` | no (secret) | At least 32 random characters. When set, `/v1/pages` (the whole catalogue) answers only `Authorization: Bearer <token>`. The website deploy sends it from its `PAGES_TOKEN` secret. |
| `GOOGLE_REDIRECT_URI` | no | Defaults to `<WEB_ORIGIN>/v1/auth/google/callback`. |
| `SECURE_COOKIES` | no | Defaults to `true`. Set `false` only for local http. |
| `DB_MAX_CONNECTIONS` | no | Defaults to 5. |
| `PORT` | no | Defaults to 8080. Most hosts set it. |

Health check: `GET /v1/health` (needs no proxy secret).

## Database

`deploy/db-copy.sh SOURCE_URL TARGET_URL` copies a whole database with `pg_dump` and
`pg_restore`. It is how data first goes to the host, and how it moves between providers
(Neon to RDS, for example). The data importer (`nirog-importer`) can also load a fresh database
from source files instead.

## Hosts

| Host | How |
|---|---|
| Google Cloud Run (current) | `deploy/gcp/cloud-run.sh`. Builds with Cloud Build, keeps secrets in Secret Manager. Run `deploy/gcp/budget-guard.sh` first: see Cost guardrails. |
| AWS App Runner or ECS Fargate | Push the same image to ECR, create the service on port 8080, and pass the settings above, with secrets from Secrets Manager. Point `API_ORIGIN` in the website at the new address. |
| Any VM | `docker run -p 8080:8080 --env-file prod.env nirog-api` |

## Releases and rollback

Merging to `main` deploys automatically once CI passes (`.github/workflows/deploy.yml`, which runs `deploy/gcp/cloud-run.sh`). GitHub signs in to Google with Workload Identity Federation, so no key is stored anywhere: `deploy/gcp/github-deploy-setup.sh` set it up once, and only this repo's `main` branch can use it.

Each deploy is a release with a [semantic version](https://semver.org) such as `v1.0.1`: a Git tag, a [GitHub Release](https://github.com/iaintsaket/nirog-api/releases) listing its PRs, and the Cloud Run revision `nirog-api-v1-0-1`. A merge bumps the patch number; label the PR `release: minor` or `release: major` to bump those.

To go back, open **Actions → rollback → Run workflow** and enter a version, or run `PROJECT=nirog-prod deploy/gcp/rollback.sh v1.0.0`. All traffic moves to that revision in seconds, with no rebuild, and GitHub's "Latest" badge moves to it. This works for the 5 newest releases, because older images are cleaned up to stay in the free storage. The next deploy sends traffic to the new release again. Then revert the bad PR so `main` matches what is live.

## Cost guardrails

The site is meant to cost nothing. Guardrails, not good intentions, keep it there:

- **Neon and Cloudflare** are on free plans with no card. Over a limit they stop serving; they cannot charge.
- **Google Cloud** has a card, so `deploy/gcp/budget-guard.sh` sets a monthly cap (default 50 in the billing currency). It emails at 20%, 50% and 90%, and at 100% a function (`deploy/gcp/budget-guard/`) unlinks billing from the project, which stops the API. Relink billing in the console to bring it back. Google reports cost a few hours late, so spend can slightly overshoot.
- **Usage ceilings:** one Cloud Run instance at most, CPU billed only during requests, 30-second timeout. The API refuses requests without the proxy secret, so all traffic passes Cloudflare's free daily request limit first.
- **Free allowances:** builds use the default `e2-standard-2` machine, Artifact Registry keeps only the 2 newest images, and there are 4 secrets (Secret Manager is free up to 6 active versions).
- Outbound data from Singapore is the one usage-based charge with no free allowance; at this traffic it is a few rupees a month, under the cap.

Moving hosts changes only these scripts and the settings, never the application code.
