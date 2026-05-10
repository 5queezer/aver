# 20. Browser consent flow replacing approval_token

Date: 2026-05-10

## Status

Proposed

## Context

ADR-0015 established the MCP/OAuth server. The current `/oauth/authorize`
endpoint (`crates/aver-server`) is gated by an `AVER_LOCAL_AUTHORIZATION_TOKEN`
environment variable: callers must supply `approval_token=<secret>` as a query
parameter or the endpoint returns 401. README:145-156 documents this as
intentional — public dynamic-client registration must not be able to
self-authorize memory access.

The gate works, but it has three concrete problems:

1. **No standard OAuth client passes `approval_token`.** It is a non-standard
   query parameter. MCP clients that handle OAuth (VS Code native MCP,
   MCP4Humans, Cursor, Claude Desktop) follow RFC 6749 / RFC 7636 and do not
   know about it. So today no compliant MCP client can complete the flow
   end-to-end against Aver. Users have to mint tokens with curl and paste them
   as static `Authorization: Bearer` headers.
2. **It is a shared secret in env, not a consent decision.** Anything that
   can read the environment of `aver-server` (a child process, a leaked
   service file, a misconfigured systemd dump) can authorize itself silently.
   There is no record of which client was approved when.
3. **It does not scale beyond localhost.** In a Docker or Cloud Run
   deployment the env-var pattern provides only authentication-by-shared-secret,
   which is exactly what platform-level auth (IAP, IAM, Cloudflare Access)
   already does better. Inside Aver we should be making the *consent* decision,
   not duplicating the auth decision.

The pattern Google and GitHub use for the same problem is a browser consent
screen at `/oauth/authorize`. Aver is the OAuth provider — it owns the data —
so Aver must be the one rendering that screen. This ADR specifies that flow.

## Decision

Replace `approval_token` with a browser-rendered consent screen at
`/oauth/authorize`. Authentication of the user is layered separately and
varies by deployment profile.

### Flow

1. MCP client redirects the user's browser to:

   ```http
   GET /oauth/authorize?
       response_type=code
       &client_id=<dynamic-registration-id>
       &redirect_uri=<client-uri>
       &code_challenge=<S256>
       &code_challenge_method=S256
       &scope=<requested-scopes>
       &state=<client-state>
   ```

2. Aver checks the deployment profile (see below) to determine whether the
   request is authenticated. If not, it redirects to its login route, then
   returns to `/oauth/authorize` with the same parameters.

3. Aver renders an HTML consent page showing:

   - The dynamic `client_id`, `redirect_uri`, and registration timestamp.
     (Dynamic-registered clients have no human-curated name or logo, so the
     page must make that visible — the user is the only line of defense
     against a malicious self-registration.)
   - The requested scopes, mapped to Aver's tool surface (read claims,
     write claims, record events, etc.).
   - Approve / Deny buttons.
   - Optional "remember this client" checkbox to skip the screen on future
     authorizations from the same `client_id` (still re-prompted on scope
     change).

4. On Approve, Aver:
   - Mints an authorization code bound to `(client_id, redirect_uri,
     code_challenge, scopes, user_id)` with the existing single-use semantics
     from ADR-0015.
   - Records a `client_consent` row in the auth DB with `granted_scopes`,
     `granted_at`, and `last_used_at`.
   - Redirects to `redirect_uri?code=...&state=...`.

5. The MCP client exchanges the code at `/oauth/token` (unchanged from
   ADR-0015).

`approval_token` and `AVER_LOCAL_AUTHORIZATION_TOKEN` are removed.

### Deployment profiles

The consent screen is identical across profiles. What changes is who
authenticates the human before the screen is shown.

#### Profile A — Localhost

The bind address is `127.0.0.1`. The fact that a request reached the local
browser is the proof of identity (no other user on a single-user machine can
trigger one).

- No login required. Aver treats any loopback request with a same-origin
  cookie as authenticated for a single fixed `user_id = "local"`.
- The consent screen still renders on every new `client_id` — that is what
  defends against DNS-rebinding (a malicious page hitting `127.0.0.1:3317`
  would have to also click Approve in the user's browser).
- A `Sec-Fetch-Site` / `Origin` check rejects cross-site `POST /oauth/authorize`
  to harden against rebinding.

#### Profile B — Docker on the user's machine

Identical to Profile A as long as the container publishes only to
`127.0.0.1:3317`. `AVER_BASE_URL` must match what the browser sees (typically
`http://127.0.0.1:3317`) so OAuth metadata, redirect URIs, and the consent
form action all align.

#### Profile C — Public deployment (Cloud Run, VPS, etc.)

The consent screen is unchanged but a real authentication boundary appears
in front of it.

- **Recommended**: terminate authentication at the platform (Cloud Run with
  `--no-allow-unauthenticated` and IAM, Cloudflare Access, Tailscale, IAP).
  Aver receives a signed identity header and trusts it. No login UI inside
  Aver. Consent screen still renders.
- **Alternative**: Aver runs its own login. Either local credentials
  (password / passkey stored in the auth DB) or federated identity ("Sign in
  with Google / GitHub"). In the federated case, the upstream IdP is used
  only to identify the user — Aver still owns the consent decision and mints
  its own access tokens. The user-facing flow is: Aver login screen → Aver
  consent screen → redirect to MCP client.

The boundary distinction matters: Aver never accepts upstream IdP tokens as
MCP access tokens. The MCP `Authorization: Bearer` always carries an Aver-issued
token, so revocation, scope, and audit stay inside Aver regardless of how the
user signed in.

### Scopes

Replace the implicit "all access" model with explicit scopes mapped to the
ADR-0008 / ADR-0015 tool groups:

| Scope                | Tools                                                              |
|----------------------|--------------------------------------------------------------------|
| `claims:read`        | `recall`, `expand`                                                 |
| `claims:write`       | `remember_claim`, `add_triple`, `contradict`, `consolidate`        |
| `events:write`       | `record_event`, `should_extract_memories`                          |
| `candidates:manage`  | `propose_claims`, `list_candidate_claims`, `promote_*`, `reject_*` |
| `observations:read`  | `recall_observation`, `compaction_summary`                         |
| `observations:write` | `record_observation`                                               |

Tools enforce scopes at the MCP boundary; missing scope returns the
ADR-0015 unsupported-scope error. Consent screen lists requested scopes in
plain language ("Allow this client to write claims to your memory.").

### Auth DB additions

ADR-0015's `AuthDb` gains:

- `users(id, kind, external_id, created_at)` — `kind` is `local` for
  Profile A/B, `header` / `oidc:google` / etc. for Profile C.
- `client_consents(user_id, client_id, granted_scopes, granted_at,
  last_used_at, revoked_at)` — durable record of approvals.
- `sessions(id, user_id, created_at, expires_at)` — server-side session
  cookies for the consent screen, separate from MCP access tokens.

Existing tables (clients, codes, access tokens, refresh tokens) remain.

### What this does not change

- Token format, hashing, and exchange semantics from ADR-0015 are unchanged.
- PKCE S256 remains required.
- `/oauth/register`, `/oauth/token`, `/.well-known/oauth-authorization-server`
  remain. Discovery metadata gains `scopes_supported`.
- The MCP route still validates `Authorization: Bearer` and resolves it to a
  `(user_id, client_id, scopes)` triple before tool dispatch.
- `aver-cli` and the local `Store` API are unaffected.

## Consequences

- (+) Standard OAuth-capable MCP clients (VS Code native MCP, Cursor,
  Claude Desktop, MCP4Humans) work end-to-end with no curl pre-step.
- (+) Authorization is a recorded consent decision, not a shared secret.
  Per-client revocation becomes a row update.
- (+) Scopes give the user real meaning to approve, and tools real meaning
  to deny.
- (+) The same flow works in localhost, Docker, and Cloud Run; only the
  outer authentication layer changes.
- (+) Removes `AVER_LOCAL_AUTHORIZATION_TOKEN` and the env-leak threat that
  comes with it.
- (-) Implementation cost: HTML consent template, server-side session
  cookies, optional federated-IdP integration, scope enforcement in the
  MCP service, and a small UX iteration to keep the screen unobtrusive.
- (-) New attack surface: the consent screen is now a CSRF target and a
  phishing target (a malicious local page could iframe or pop-open
  `/oauth/authorize` and try to social-engineer Approve). Mitigations:
  `X-Frame-Options: DENY`, anti-CSRF token on the form, `Sec-Fetch-Site`
  checks, scope-by-scope opt-in.
- (-) Profile-C deployments must integrate with whatever upstream auth
  layer they choose; Aver does not ship a one-click Cloud Run config.
- (-) Existing local users with a working `AVER_LOCAL_AUTHORIZATION_TOKEN`
  setup must re-approve clients once after the upgrade. One-shot, no data
  migration.

## Implementation notes

Suggested slicing, smallest first:

1. Add `users`, `client_consents`, `sessions` tables and a
   `Sec-Fetch-Site` / `Origin` validator.
2. Implement Profile A: loopback bypass + HTML consent screen +
   anti-CSRF token + form `POST /oauth/authorize/decision` that mints the
   code on Approve.
3. Add `scopes_supported` to discovery metadata and scope enforcement in
   the MCP service.
4. Add Profile C trust-header path (`AVER_TRUSTED_AUTH_HEADER=X-Forwarded-User`)
   so Cloud Run / IAP deployments work without Aver-internal login.
5. Add Profile C local-credential login (passkey first, password second)
   for self-hosted public deployments.
6. Remove `AVER_LOCAL_AUTHORIZATION_TOKEN` and the `approval_token`
   query handling. Update README:145-156.
7. Federated IdP login (`Sign in with Google/GitHub`) as a follow-up ADR
   if Profile C usage warrants it.

Slices 1-3 unblock the local MCP-client UX that motivated this ADR.
Slices 4-6 close out the public-deployment story.
