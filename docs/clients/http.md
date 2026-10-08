---
title: HTTP
description: The routes a client or a deploy talks to, and who may call them.
---

HTTP is the contract every client uses. The JavaScript, Swift, and Kotlin packages are thin wrappers over these routes. Send `Authorization: Bearer` unless the row says otherwise. Project routes need a project, from the host or from the token.

A success body is the resource. An error body is `{ "error": "..." }` with an HTTP status. A session from signup or password login:

```json
{
  "access_token": "eyJ...",
  "refresh_token": "opaque-token",
  "user": { "id": "6b1c1a4e-2f0a-4d3b-9c11-0a9e8d7c6b5a", "email": "ada@example.com" }
}
```

Over an auth rate limit:

```json
{ "error": "too many requests" }
```

That response is status 429 and includes `Retry-After: 60`. Every response, including errors, includes `x-request-id`.

## Platform

No project. The operator token is not a project key.

| Method | Path | Who | Effect |
| --- | --- | --- | --- |
| `GET` | `/health` | anyone | Postgres and the blob store are reachable |
| `GET` | `/platform/v1/extensions` | operator | Extensions enabled on this cluster, `{ name, routes }[]` |
| `GET` | `/platform/v1/projects` | operator | List refs |
| `POST` | `/platform/v1/projects` | operator | Create a project. Returns ref, anon key, service key |
| `PATCH` | `/platform/v1/projects/{ref}` | operator | Update the project row |
| `POST` | `/platform/v1/migrate` | operator | Apply `sql/project/` |
| `POST` | `/platform/v1/domains/{host}/verify` | operator | Mark a domain verified |
| `GET` | `/ask?domain=` | proxy | 200 when that custom domain is verified, otherwise 404 |

## Auth

| Method | Path | Who | Body |
| --- | --- | --- | --- |
| `POST` | `/auth/v1/signup` | anon key | `{ email, password }` → session, or `{ verification_required, user }` |
| `POST` | `/auth/v1/token` | anon key, or the refresh token in the body | `{ email, password }`, `{ refresh_token }`, or `{ code }` → session, or a verification or 2FA body |
| `POST` | `/auth/v1/verify-email` | anon key | `{ token }` or `{ email, code }` → session |
| `POST` | `/auth/v1/verify-email/send` | anon key | `{ email }` → `{ ok: true }` |
| `POST` | `/auth/v1/factors/totp` | anon key | `{ mfa_token, code }` → session |
| `POST` | `/auth/v1/factors/passkey/verify` | anon key | credential plus `mfa_token` → session |
| `POST` | `/auth/v1/factors/recovery` | anon key | `{ mfa_token, code }` → session |
| `GET` | `/auth/v1/authorize` | anon key | `provider` and `redirect_to` query → redirect |
| `GET` | `/auth/v1/callback/{provider}` | browser | provider code → redirect with `code` or `error` |
| `POST` | `/auth/v1/logout` | the refresh token in the body | `{ refresh_token }` → 204 |
| `GET` | `/auth/v1/user` | user access token | `{ id, email, email_verified_at }` |
| `POST` | `/auth/v1/magic-link` | anon key | `{ email }` → `{ ok: true }` |
| `POST` | `/auth/v1/verify` | project | `{ token }` → session |
| `POST` | `/auth/v1/recover` | anon key | `{ email }` → `{ ok: true }` |
| `POST` | `/auth/v1/recover/complete` | project | `{ token, password }` → session |
| `POST` | `/auth/v1/invite` | service key | `{ email }` → `{ ok: true }` |
| `POST` | `/auth/v1/invite/accept` | project | `{ token, password }` → session |

A session is `{ access_token, refresh_token, user: { id, email } }`.

Signup, magic-link, recover, and invite are limited to 30 requests a minute per project and client IP. Password token is 60 a minute. Over the limit the response is 429 with `Retry-After: 60` and `{ "error": "too many requests" }`. Refresh is not counted. The client IP is the socket peer. `X-Forwarded-For` is used only when `http.trusted_proxy` is set.

`OPTIONS` on `/auth/v1`, `/data/v1`, `/storage/v1`, and `/fn/v1` returns 204. A response echoes `Origin` when the host is `localhost`, `127.0.0.1`, `*.{base_domain}`, or a verified domain. Allowed request headers are `Authorization`, `Content-Type`, `apikey`, `Prefer`, `Range`, and `Accept`. An unknown origin gets no allow-origin header.

Every response includes `x-request-id`. Each process logs its own requests. There is no hosted metrics product.

`GET /health` is 200 with an empty body when Postgres accepts `SELECT 1`, and 503 when it does not. It does not return JSON.

Creating a project with the operator token returns the keys once:

```json
{
  "id": "019f0000-0000-7000-8000-000000000000",
  "ref": "abcdefghijklmnopqrst",
  "anon_key": "eyJ...",
  "service_key": "eyJ..."
}
```

## Data, storage, functions

| Method | Path | Who |
| --- | --- | --- |
| any | `/data/v1/...` | anon, user, or service. Proxied to PostgREST |
| `POST` | `/storage/v1/object/presign` | a project token. Body `{ bucket, key, method }` |
| `GET` or `PUT` | `/storage/v1/signed` | the signature on the query string |
| `POST` | `/fn/v1/{name}` | a project token. Body is stdin. Names starting with `_` are not public |
| `POST` | `/fn/v1/{name}/enqueue` | service key. `{ body, delay_secs, max_attempts }` → 201 `{ id }` |
| `GET` | `/fn/v1/_admin/tasks/{id}` | service key, same project. `{ id, kind, status, attempts, max_attempts, last_error }` |
| `POST` | `/fn/v1/_admin/functions/{name}` | service key. Body is a zip. `?promote=false` keeps the previous version live |
| `POST` | `/fn/v1/_admin/schedules` | service key. `{ function_name, body }` |
| `POST` | `/fn/v1/_internal/cron` | operator token |
| `POST` | `/_internal/tick` | operator token. `?force=1` ignores hook intervals |

## Queue

Only when `REACTOR_EXTENSIONS` includes `queue`. Service key. The routes and bodies are on [Queue](/product/queue/).

## Sites

Service key, except the public site host itself.

| Method | Path |
| --- | --- |
| `POST` | `/sites/v1/deployments` |
| `POST` | `/sites/v1/deployments/{id}/files/{path}` returns `{ "url", "fallback" }` |
| `PUT` | `/sites/v1/deployments/{id}/files/{path}` body, at most 10 MB in listen mode |
| `POST` | `/sites/v1/deployments/{id}/files/{path}/confirm` |
| `POST` | `/sites/v1/deployments/{id}/finish` |
| `POST` | `/sites/v1/deployments/{id}/fail` |
| `PUT` | `/sites/v1/files/{path}` |
| `GET` or `PUT` | `/sites/v1/env` |
| `POST` | `/sites/v1/routes` with `{ path, function }` |
| `POST` | `/sites/v1/domains` with `{ host }` |
| `POST` | `/sites/v1/domains/{host}/verify` |
| `DELETE` | `/sites/v1/domains/{host}` |

On `{ref}.{base_domain}`, `GET` and the other methods are the site. `/` serves `index.html`.

## Console

`/console/v1` uses a console token. The pages and role rules are in [Console](/operate/console/). The first call is `GET /console/v1/setup`, then `POST /console/v1/setup` or `POST /console/v1/login`.
