# PRD-1: Where to buy — availability, delivery time and a vendor section

Covers requests 1, 4 ("hide medicines out of stock everywhere") and 5.

## Problem
Today each offer carries 2–4 "buy links" rendered as small chips (`CompareDialog.tsx`, `MedicineTile.tsx`).
They are search URLs built from the name (`nirog-api/crates/application/src/sellers.rs`). Nirog does not know
whether the vendor stocks the medicine, its price there, or how fast it arrives. Users click through blind.

## Goals
1. For each offer, show per vendor: **in stock / out of stock / unknown**, vendor price, and delivery estimate (TAT).
2. Recommend one vendor: in stock, then fastest TAT, then cheapest. Show why ("Arrives tomorrow, ₹12 cheaper").
3. Hide an offer that is out of stock at **every** known vendor (and not a Jan Aushadhi generic, see below).
   "Unknown" is never treated as out of stock.
4. Replace the chips with a dedicated **Where to buy** section: one row per vendor, status pill, price, TAT,
   and last checked time. Out-of-stock rows are greyed and sorted last.

## Non-goals
Placing orders, carts, payments, or comparing coupons.

## Data
Online pharmacies offer no public stock or TAT API. Build the **port and UI now** behind a `StockSource` trait
that reads stored listings, so a partner API plugs in as one more impl + one registry line when one is available.
Sources are named generically (`source_a`..`source_d`).

Jan Aushadhi generics cannot be sold online; their availability is "at Kendras" and links to PRD-4's map.

## Design
- API: new port `StockSource { fn check(&self, offer, pincode?) -> Vec<VendorStatus> }`;
  `VendorStatus { vendor, status: InStock|OutOfStock|Unknown, price_paise?, tat_days?, checked_at }`.
- Ranking (`recommend`) is pure, tested domain code — no LLM.
- `/v1/compare` adds `vendors: VendorStatusDto[]` per offer and `recommended_vendor`; `links` kept one release for compatibility.
- Filtering of all-out-of-stock offers happens in the API so the static medicine pages agree with the app.
- Web: `WhereToBuy.tsx` section in the compare view and dialog; optional pincode input (stored locally only).

## Acceptance
- Offer with every vendor OutOfStock is absent from `/v1/compare` and `/v1/search`; with one Unknown it stays.
- Recommended vendor follows in-stock → TAT → price, tie-break by name; unit tested.
- Where-to-buy section passes the existing Playwright phone + desktop layout checks; out-of-stock is legible without colour.
- "Last checked" is always visible; data older than 7 days says so.
