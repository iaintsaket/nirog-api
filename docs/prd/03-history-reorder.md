# PRD-3: Search history and reorder (signed-in users)

Covers request 3. Sign-in exists (Google) but is hidden at launch (`NEXT_PUBLIC_SIGN_IN`); saved matches exist.

## Goals
1. Signed-in users get a **History** page: medicines searched/compared, newest first, deduplicated per match key,
   with today's cheapest price and change since last viewed.
2. **Reorder**: one tap opens Where to buy (PRD-1) for that medicine, preselecting the last vendor used
   (we record the Buy click, not the purchase). "Reorder all" for a scanned prescription (PRD-2).
3. Controls: pause history, delete one, clear all; account deletion (`DELETE /me`) removes it.

## Design
- Table `search_history(user_id, match_key, offer_id, last_vendor, viewed_at)`, unique (user_id, match_key), capped at 200 rows per user.
- Recorded server-side on `/v1/compare` only when a session cookie is present; never for anonymous users.
- Endpoints: `GET /v1/history`, `DELETE /v1/history/{id}`, `DELETE /v1/history`, `POST /v1/history/{id}/buy-click`.
- Privacy page updated: history is health-adjacent data; retained 12 months.

## Decision needed
**Decided (2026-11-24):** ship behind `NEXT_PUBLIC_SIGN_IN`; sign-in stays off until the owner turns it on.

## Acceptance
- Anonymous requests never write history (test). Delete-me cascades (migration test).
- Reorder preselects the last vendor; History shows the price change.
