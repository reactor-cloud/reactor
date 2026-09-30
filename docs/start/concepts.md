---
title: Concepts
description: Tenancy, identity, and how one Reactor process serves many projects.
---

This page is the model behind every other page. A cluster is one Reactor process, Postgres, and a blob store. The process does not remember a current tenant between requests. Every request resolves a project, then the database transaction is limited to that project's schema. Project users and console operators are different identities, and they cannot use each other's tokens.

If the hostname and the token name different projects, the response is 403:

```json
{ "error": "host and token project mismatch" }
```

## One server

```text
Internet → TLS proxy
             → reactor
                  ├─ Postgres
                  ├─ PostgREST
                  └─ blobs (filesystem or S3)
```

The TLS proxy is Caddy, Fly, CloudFront, or API Gateway. Its config is static. Creating a project does not push a route into it. Platform hosts `{ref}.{base_domain}` are covered by a wildcard certificate.

The container image and the Lambda handlers are the same routers. `runtime.mode` is `listen` or `lambda`. In Lambda, `REACTOR_HANDLER` selects `auth`, `storage`, `sites`, `fn`, or `platform`. Data is PostgREST, not the Reactor binary.

## Tenancy

A request with no project is refused, except health, project creation, migration, and the custom-domain check.

Resolution order:

1. Hostname `{ref}.{base_domain}`, or a verified row in `reactor.domains`.
2. The token's `ref` claim.
3. When both are present they must be the same project, or the response is 403.

| Concern | Where it lives |
| --- | --- |
| Control data | schema `reactor` |
| Tenant tables | schema `proj_<ref>` |
| Dedicated database | `reactor.projects.database_url`, when set |
| Objects | `{ref}/{bucket}/{key}` |
| Functions, sites, keys | rows with `project_id` |

Creating a project creates the schema, the grants, the anon key, and the service key. Migrations in `sql/project/` apply to every project schema. A migration cannot land on only the project you happen to be using.

## Two identities

A **project user** signs up through `/auth/v1`. The access token has role `authenticated`. PostgREST and row-level security accept that token.

A **console operator** signs in through `/console/v1`. That token has audience `console`. It cannot call auth or data. A project user token cannot call the console.

API keys are long-lived tokens with role `anon` or `service`, bound to one project. The anon key is what a browser sends before anyone signs in. The service key bypasses row-level security. It is for deploys and trusted server code.

## Data enforcement

Postgres row-level security is the data boundary. Tables start with row-level security enabled and forced, and the default is deny. Roles:

| Role | What it can do |
| --- | --- |
| `authenticator` | PostgREST logs in as this role and `SET ROLE`s per request. `NOINHERIT`. |
| `anon` | Usage on the project schema. RLS applies. |
| `authenticated` | Same, for a signed-in user. |
| `service` | `BYPASSRLS`. |

`reactor.pre_request()` runs inside the request transaction. It reads the JWT claims, checks `ref`, and sets `search_path` to `proj_<ref>` only. The authenticator role has an empty search path, so a request that skips the pre-request sees no tables.
