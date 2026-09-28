---
title: Projects
description: Refs, schemas, API keys, hostnames, and a dedicated database.
---

A project is the tenant. Creating one creates a schema, grants, an anon key, and a service key.

## Ref and schema

The ref is a random id, not the display name. The schema is `proj_<ref>`. Product tables live there, applied by `reactor db migrate` or by `reactor deploy`.

Control tables stay in the `reactor` schema: projects, domains, API key hashes, users, sessions, functions, schedules, sites, operators, and memberships.

## Keys

| Key | Role | Who holds it |
| --- | --- | --- |
| Anon | `anon` | Browsers and mobile apps, before sign-in |
| Service | `service` | Deploys and trusted server code. Bypasses row-level security |
| User access token | `authenticated` | A signed-in project user. Lives about 15 minutes |
| Refresh token | — | A row in Postgres, hashed. Rotates on use. Expires in 30 days |
| Console token | `console` | An operator. Rejected by auth and data |
| Operator token | — | Platform routes such as migrate. Not a project key |

Keys are hashed in `reactor.api_keys`. Plaintext is returned only when a project is created or keys are rotated (`reactor keys rotate`). Rotating rewrites `.reactor/service_key` when that file exists.

Send keys as `Authorization: Bearer`.

## Hostname

Project routes resolve the tenant from the host, then from the token.

- `http://{ref}.apps.localhost:18000` is the local platform host. `base_domain` defaults to `apps.localhost`.
- A custom domain is a row. After DNS verification, `GET /ask?domain=` returns 200 and the proxy may issue a certificate. Reactor does not terminate that TLS.
- If the host and the token name different projects, the response is 403.

API calls can use the platform host or the API origin in `reactor.toml`, as long as the token's ref matches the host when both are present.

## Dedicated database

Set `database_url` on the project and callers do not change. The connection resolver opens the shared database or that URL, then the transaction sets the search path to the project schema only. Local Compose includes a second Postgres for this.

## Delete

An owner can delete a project from the console. That drops the schema, the blobs under the ref, and the project row. The console asks for the project name before it deletes.
