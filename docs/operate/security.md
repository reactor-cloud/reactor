---
title: Security
description: Row-level security, tokens, function isolation, and what clients cannot reach.
---

Security here is what the server enforces before your policies do. Tokens name a project and a role. Postgres row-level security decides which rows that role can see. The console, the service key, and the operator token are outside that boundary on purpose, and they do not belong in a client app.

## Data

Row-level security is enabled and forced on tenant tables that ship with the server, and that is the pattern for tables you add. The default, with no policy, is deny. The service role is the only bypass.

Tokens are verified in one place. The database role does the rest. `anon`, `authenticated`, and `service` are the roles PostgREST will assume. A console token is not one of them.

API keys and user tokens carry `ref`. When the host also names a project, the two must match.

Postgres is not exposed to clients. They speak HTTP to Reactor, and to PostgREST through `/data/v1`.

## Functions

A function process starts with a cleared environment. It receives the caller JSON and its own variables. It does not receive the server database URL or the service key unless you stored those as that function's variables. Do not do that.

Zip entries that contain `..` or start with `/` are rejected.

## Storage

Presign checks the project prefix before it issues a URL. The signed URL expires in 300 seconds. Filesystem URLs are HMAC'd with `storage.sign_secret`.

## Operator routes

`POST /platform/v1/projects`, migrate, and `POST /fn/v1/_internal/cron` require the operator token. That token is not a project key. Console routes require a console session and a membership. Rotating keys and deleting a project are limited to admin and owner.

## Email

Magic-link and recovery responses are `{ "ok": true }` whether or not the address exists and whether or not the message was sent. Invite is the exception: it needs the service key, and it fails when email is not configured.

Challenge tokens are stored hashed. A magic link or recovery token lasts 15 minutes. An invite lasts 7 days. Repeating a send inside 60 seconds does not create another challenge.

Signup, magic-link, recover, and invite accept 30 attempts a minute per project and client IP. Password login accepts 60. The response over the limit is 429 with `Retry-After: 60`:

```json
{ "error": "too many requests" }
```

The address is the socket peer unless `http.trusted_proxy` is set, in which case the first `X-Forwarded-For` value is used. Refresh is not counted.

Browser clients may call `/auth/v1`, `/data/v1`, `/storage/v1`, and `/fn/v1` from `localhost`, `127.0.0.1`, a host under `base_domain`, or a verified domain. Other origins are not reflected in `Access-Control-Allow-Origin`.

Each response carries `x-request-id`. Scrape each process on its own.

## What you still have to do

Reactor does not write your policies for you. A table without `ENABLE` and `FORCE ROW LEVEL SECURITY`, or a policy of `USING (true)` granted to `anon`, is public. The console grid bypasses user policies on purpose. It is not a test of them.

The operator token, the service key, the JWT private key, and storage credentials are cluster secrets. They do not belong in a client or in git.
