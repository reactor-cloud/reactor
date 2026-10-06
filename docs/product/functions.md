---
title: Functions
description: Bun or Lambda functions, versions, pins, variables, and cron.
---

Functions are server-side code for one project. A function is a zip in the blob store and a row in Postgres: name, project, version, object key, runtime. `POST /fn/v1/{name}` runs the live version. Clients send JSON and read JSON. They do not receive the database URL or the service key unless you stored those as that function’s own variables.

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

The HTTP response is `application/json` and the stdout bytes. The example above answers:

```json
{ "ok": true, "caller": { "sub": "…", "ref": "…", "role": "authenticated" }, "req": {} }
```

A non-zero exit is status 500 and the stderr text. The Bun invoke timeout is 30 seconds. The todos `ping` function adds `banner`, which is null unless that name is one of the function's own variables.

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

`POST /fn/v1/_internal/cron` with the operator token takes a Postgres advisory lock, invokes every schedule that is due, and sets `next_run` to one day later. The caller JSON is `{ "sub": "cron", "ref": "<ref>", "role": "service" }`. If the lock is already held, the response is `{ "ran": false }` and nothing else runs. Listen mode also runs this about once a minute. On Lambda, call this route or [`POST /_internal/tick`](/product/extensions/#when-work-runs). A failed invoke fails that cron pass. The schedule does not retry on its own.

## Enqueue

Enqueue stores one run of a function and lets the server call it later. It is always available. It is not a [queue](/product/queue/).

```http
POST /fn/v1/{name}/enqueue
Authorization: Bearer <service key>
Content-Type: application/json

{ "body": { "ok": true }, "delay_secs": 0, "max_attempts": 3 }
```

The response is 201 `{ "id" }`. `body` is the function stdin and defaults to `{}`. `delay_secs` defaults to 0. `max_attempts` defaults to 3. A negative delay or `max_attempts` below 1 is 400. Names that start with `_` are 404.

```http
GET /fn/v1/_admin/tasks/{id}
```

The same service key, and the same project, returns `{ "id", "kind", "status", "attempts", "max_attempts", "last_error" }`. Another project's id is 404. `status` is `queued`, `running`, `done`, or `dead`.

Listen mode claims due tasks about once a second. A task that is still `running` after a 30 second lease can be claimed again. A failed attempt waits 2 seconds, then 4, 8, 16, 32, and then 60, and goes back to `queued` while `attempts` is below `max_attempts`. The next failure after that is `dead`. A successful run is `done`. Done rows are removed after 24 hours.

The function log is written when the task actually runs, with kind `function`. Enqueue itself does not write that row.

The clients call this `functions.enqueue(name, { body, delaySecs, maxAttempts })` and `functions.task(id)`. Pass the service key as the client key.

## Logs

Invoke appends a `reactor.logs` row with kind `function`. A failed log write does not fail the request. The console and `reactor logs` show recent lines.
