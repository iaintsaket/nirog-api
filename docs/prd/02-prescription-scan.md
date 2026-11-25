# PRD-2: Scan a prescription → exact and cheaper substitutes for every medicine

Covers request 2.

## Goal
User uploads or photographs a prescription. Nirog lists each medicine it read, matched to the catalog, with the
exact item, the cheapest exact substitute, and (once PRD-1 lands) the one that arrives first.

## Flow
1. `/scan` page: take photo / choose image or PDF. Image never leaves the device by default.
2. OCR runs **in the browser** (Tesseract.js, English, lazy-loaded) → raw text shown to the user, editable.
3. Text → candidate lines → tested matcher (code, not an LLM): strip dosing words (`1-0-1`, `BD`, `x 5 days`,
   `Tab.`, `Cap.`), fuzzy-match brand/generic names and strengths against `/v1/search`.
4. Result table: what we read · matched medicine (user confirms or picks another) · cheapest exact substitute · saving · Where to buy.
5. "Compare all" total: what the prescription costs as written vs cheapest exact substitutes.

## Decision needed: OCR engine
| Option | Handwriting | Privacy | Cost |
|---|---|---|---|
| A. Tesseract.js in the browser | Poor; fine on printed/typed Rx | Image stays on device | Free |
| B. Cloud vision (Google Vision / Claude) | Good | Health image sent to a third party; needs consent + privacy page update | Per call; fights the ₹50 cap |
**Decided (2026-11-24): A (Tesseract.js in the browser)** first, with clear "printed prescriptions work best; edit what we read" copy. B later behind consent.

## Non-goals
Interpreting dosage, advising substitutions, storing prescription images.

## Acceptance
- Matcher unit tests on ≥20 real-style Rx lines (typed + noisy OCR) reach ≥85% top-1 match.
- No network request carries the image (asserted in a Playwright test).
- Unmatched lines are shown, never silently dropped. Disclaimer: not medical advice.
