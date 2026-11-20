# nirog-api

The API behind **Project Nirog**: find cheaper, exact substitutes for a medicine in India,
from Jan Aushadhi generics and branded alternatives, with the same salts, strength, form and release.

The web app lives in [iaintsaket/nirog-web](https://github.com/iaintsaket/nirog-web).

> Nirog compares prices only. It is not medical advice. Ask your doctor or pharmacist before switching.

## Architecture

A Rust workspace in ports-and-adapters style:

| Crate | Role |
|---|---|
| `crates/domain` | Medicines, salts, strengths, the exact-match key and price comparison. No I/O. |
| `crates/application` | Use cases (search, compare, sign-in, saved matches) and the ports they depend on. |
| `crates/persistence` | Postgres adapters for the ports (sqlx, migrations). |
| `crates/identity-google` | Google OAuth (code + PKCE) adapter. |
| `crates/sources` | Price-list readers (Jan Aushadhi CSV, brand dataset CSV). |
| `crates/http` | axum `/v1` API, security middleware, OpenAPI spec. |
| `apps/server` | The HTTP server binary. |
| `apps/importer` | Loads price lists into Postgres. |

The API contract is [`openapi.json`](openapi.json). A test fails if it drifts from the code;
regenerate it with `UPDATE_OPENAPI=1 cargo test -p nirog-http --test api openapi`.

## Run locally

```bash
docker compose -f deploy/docker-compose.yml up -d db   # Postgres 17
cp .env.example .env                                   # then fill in Google OAuth values
export DATABASE_URL=postgres://nirog:nirog@localhost:5432/nirog
cargo run -p nirog-importer                            # bundled Jan Aushadhi list
cargo run -p nirog-importer -- --brands path/to/brands.csv   # optional brand dataset
cargo run -p nirog-server
```

## Data

- Jan Aushadhi product list and MRPs (official PMBI list, see [`data/README.md`](data/README.md)).
- Brand prices come from a public dataset that is imported at run time and never committed.

## Checks

Every pull request must pass the `ci` check: rustfmt, clippy (warnings are errors), tests on
Postgres 17 with at least 80% line coverage, migrations on a fresh database, OpenAPI drift,
cargo-deny, gitleaks and a Docker build scanned by Trivy. CodeQL runs alongside.

## License

[MIT](LICENSE)
