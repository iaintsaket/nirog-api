# Security policy

Please report vulnerabilities privately through
[GitHub private vulnerability reporting](https://github.com/iaintsaket/nirog-api/security/advisories/new).
Do not open a public issue. You will get a reply within 7 days.

## What the API does to stay safe

- Google sign-in uses the authorization code flow with PKCE; the OAuth state is bound to the browser by a `__Host-` cookie.
- Sessions are random 256-bit tokens; only their SHA-256 hash is stored. Cookies are `__Host-`, `HttpOnly`, `Secure`, `SameSite=Lax`.
- State-changing requests must come from the allowed origin.
- In production the API only answers requests that carry the proxy secret added by the web app's server-side proxy.
- Per-IP rate limits, a 16 KB body limit, request timeouts and strict security headers.
- All SQL is static and parameterised.
- CI runs cargo-deny, gitleaks, CodeQL and Trivy on every pull request.
