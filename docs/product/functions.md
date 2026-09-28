---
title: Functions
description: Bun or Lambda functions, versions, pins, variables, and cron.
---

A function is a zip in the blob store and a row in Postgres: name, project, version, object key, runtime. `POST /fn/v1/{name}` runs the live version.

## Code

The zip must contain `index.ts` at the root. Reactor runs `bun index.ts` in that directory. The request body is stdin. Write the response to stdout. A non-zero exit is a 500, and stderr is the error.

The process environment is cleared first. The child receives `PATH`, `HOME`, `REACTOR_CALLER`, and that function's own variables. It does not receive the server's database URL, the service key, or another function's variables. Names starting with `REACTOR_` in a function's variable list are dropped, except `REACTOR_PROJECT_REF` when you set it yourself.

`REACTOR_CALLER` is JSON:

```json
{ "sub": "<user id or role>", "ref": "<project ref>", "role": "authenticated" }
```

Anon and service tokens use the role name as `sub` when there is no user id. A site route that calls a function sends `sub: "site"` and `role: "anon"`.

```ts
const raw = await Bun.stdin.text()
const req = raw ? JSON.parse(raw) : {}
const caller = JSON.parse(process.env.REACTOR_CALLER || "{}")
process.stdout.write(JSON.stringify({ ok: true, caller, req }))
```

The HTTP response is `application/json` and the stdout bytes. The Bun invoke timeout is 30 seconds.

## Deploy

`reactor deploy` zips each directory under `functions/` and posts it to `POST /fn/v1/_admin/functions/{name}` with the service key. The body is the zip. `promote` defaults to true, so the new version is the one that responds.

```json
{ "name": "ping", "version": 3, "sha256": "...", "promoted": true }
```

Upload with `?promote=false` to keep the previous version live. Reactor then pins that previous version.

`reactor functions promote <name> <version>` pins a stored version. `reactor functions demote <name>` clears the pin, and the latest version responds again.

In `lambda` runtime, a promote publishes the user's zip as its own Lambda. The API Lambda does not run user code in-process.

## Variables

Each function has its own variables, stored encrypted. The value is never read back. Invoke injects only that function's set. Set them in the console on the function's Variables tab, or through `POST /console/v1/projects/{ref}/functions/{name}/env`.

## Schedules

A schedule is a row, not a cron expression. Create one with the service key:

```http
POST /fn/v1/_admin/schedules
Content-Type: application/json

{ "function_name": "ping", "body": "{\"ok\":true}" }
```

`body` is the stdin payload. It defaults to `{}`. `next_run` starts at now.

`POST /fn/v1/_internal/cron` with the operator token takes a Postgres advisory lock, invokes every schedule that is due, and sets `next_run` to one day later. The caller JSON is `{ "sub": "cron", "ref": "<ref>", "role": "service" }`. If the lock is already held, the response is `{ "ran": false }` and nothing else runs. There is no in-process timer. Call the route from cron, EventBridge, or anything else that can hold the operator token.

This is not a jobs product. There is no queue, retry policy, or dead-letter store. A failed invoke fails the cron request.

## Logs

Invoke appends a `reactor.logs` row with kind `function`. A failed log write does not fail the request. The console and `reactor logs` show recent lines.
