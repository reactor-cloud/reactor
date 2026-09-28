---
title: Data
description: PostgREST on /data/v1, with row-level security as the enforcement boundary.
---

`/data/v1` is a reverse proxy to PostgREST. The prefix is stripped before the request is forwarded, so clients keep one origin and PostgREST keeps its own paths. OpenAPI for your tables is PostgREST's. Reactor does not publish a second description of them.

## How a request is scoped

PostgREST logs in as the `authenticator` role and `SET ROLE`s to `anon`, `authenticated`, or `service` from the token. `reactor.pre_request()` then sets `search_path` to `proj_<ref>` only.

Send `Authorization: Bearer` with the anon key, a user access token, or the service key. The host and the token must name the same project.

## Queries

The usual PostgREST surface works: vertical filters, embeds, `Range`, `Prefer`, and RPC.

```http
GET /data/v1/todos?select=id,title&user_id=eq.<uuid>&order=created_at.desc
Authorization: Bearer <access token>
Accept: application/json
```

```http
POST /data/v1/todos
Authorization: Bearer <access token>
Content-Type: application/json
Prefer: return=representation

{ "title": "Ship the site", "user_id": "<user uuid>" }
```

`PATCH` updates. `DELETE` removes. A write that must be one transaction is a SQL function in the project schema, exposed as RPC:

```http
POST /data/v1/rpc/create_note_pair
Content-Type: application/json

{ "a": "first", "b": "second" }
```

The JavaScript client uses `@supabase/postgrest-js` against this prefix, so filters and embeds match that library. The Swift client implements select, insert, update, delete, `eq`, `order`, and `limit`. See [JavaScript](/clients/javascript/) and [Swift](/clients/swift/).

## Row-level security

New tenant tables should enable and force row-level security, then grant only the roles that may touch them. Without a policy, `anon` and `authenticated` see nothing. `service` bypasses row-level security.

The todos migration is the pattern:

```sql
ALTER TABLE todos ENABLE ROW LEVEL SECURITY;
ALTER TABLE todos FORCE ROW LEVEL SECURITY;

CREATE POLICY todos_owner ON todos
  FOR ALL TO authenticated
  USING (user_id::text = current_setting('request.jwt.claims', true)::json->>'sub')
  WITH CHECK (user_id::text = current_setting('request.jwt.claims', true)::json->>'sub');

GRANT SELECT, INSERT, UPDATE, DELETE ON todos TO authenticated, service;
```

`anon` is not granted here, so a signed-out client cannot read the table. Policies read `request.jwt.claims`, which PostgREST sets from the token. The user id is `sub`.

`public` has no grants for these roles. Grants name `anon`, `authenticated`, and `service` on the project schema only.

## Migrations

Put SQL in `sql/project/`. `reactor deploy` applies pending files to the linked project. `reactor db migrate --all` walks every project and needs the operator token (`reactor login --token`).

The console table view is an admin view. It reads every row in the project schema and does not apply a user's policy. Do not treat it as proof that RLS is correct. Prove that with two users, or two projects.
