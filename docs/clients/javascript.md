---
title: JavaScript
description: The @reactor-cloud/client package. Auth, PostgREST data, storage, and functions.
---

`@reactor-cloud/client` is the JavaScript client for web apps. It is modeled on the Supabase JavaScript client so code stays portable: `createClient`, `auth.signUp`, `auth.signInWithPassword`, `from(table).select()`, storage buckets, and `functions.invoke` mean the same kind of thing. Data queries are built with `@supabase/postgrest-js` and sent to Reactor’s `/data/v1`, which is PostgREST.

The package is published. This beta is `1.26.10-beta8` on the `beta` dist-tag, not `latest`.

```sh
npm install @reactor-cloud/client@beta
```

## What it covers

| Call | Server |
| --- | --- |
| `reactor.auth` | `/auth/v1` — sign up, password, magic link, recovery, invite, session |
| `reactor.from` / `reactor.rpc` | `/data/v1` — PostgREST |
| `reactor.storage.from` | `/storage/v1` — presign, then upload or download the signed URL |
| `reactor.functions.invoke` | `/fn/v1/{name}` |
| `reactor.functions.enqueue` | `/fn/v1/{name}/enqueue` — service key. `task(id)` reads `/fn/v1/_admin/tasks/{id}` |
| `reactor.queue` | `/queue/v1` — service key, and the queue extension must be on |

The client keeps the session in memory. Pass `session` when you create the client if the page should start signed in. Before sign-in, calls send the anon key. After sign-in, they send the access token.

`signUp` and `signInWithPassword` return a session, `{ verification_required, user }`, or `{ mfa_required | enrollment_required, token, factors }`. A result without `access_token` is not stored as the session.

## Create a client

The first argument is the project origin with no trailing slash. The second is the anon key.

```ts
import { createClient } from "@reactor-cloud/client"

const reactor = createClient("https://<ref>.apps.localhost:18000", anonKey, {
  session: savedSession, // optional, { access_token, refresh_token, user }
})
```

`options.fetch` replaces `globalThis.fetch` when you need a custom implementation.

## Auth

```ts
const session = await reactor.auth.signUp({ email, password })
const session = await reactor.auth.signInWithPassword({ email, password })
const user = await reactor.auth.getUser()
const session = await reactor.auth.refreshSession()
await reactor.auth.signOut()
const current = reactor.auth.getSession()
```

When the result is a session, the client stores it:

```json
{
  "access_token": "eyJ...",
  "refresh_token": "opaque-token",
  "user": { "id": "6b1c1a4e-2f0a-4d3b-9c11-0a9e8d7c6b5a", "email": "ada@example.com" }
}
```

`verifyEmail`, `resendVerification`, `verifyTotp`, `verifyPasskey`, `verifyRecovery`, `enrollTotp`, and `enrollPasskey` call the matching `/auth/v1` routes. `signInWithOAuth({ provider, redirectTo })` returns the authorize URL and navigates the browser. `exchangeCode(code)` stores the session from `POST /auth/v1/token`.

`getUser()` returns `{ "id", "email", "email_verified_at" }`. `getSession()` returns the stored session, or `null` when nobody is signed in. `refreshSession()` returns a new pair and replaces the one in memory. The previous refresh token no longer works. `signOut()` asks the server to delete the refresh row and clears the in-memory session.

Magic link, recovery, and invite:

```ts
await reactor.auth.signInWithMagicLink(email)
const session = await reactor.auth.verifyMagicLink(token)

await reactor.auth.recover(email)
const session = await reactor.auth.completeRecovery(token, password)

await reactor.auth.invite(email, serviceKey)
const session = await reactor.auth.acceptInvite(token, password)
```

`signInWithMagicLink`, `recover`, and `invite` resolve with no value. The server body is `{ "ok": true }` either way, so a missing address is not distinguishable from a sent message. `verifyMagicLink`, `completeRecovery`, and `acceptInvite` return a session.

Auth failures throw `ReactorError`. `status` is the HTTP status. `message` is the server’s `error` string.

```ts
import { ReactorError } from "@reactor-cloud/client"

try {
  await reactor.auth.signInWithPassword({ email, password })
} catch (err) {
  if (err instanceof ReactorError && err.status === 401) {
    // unknown email or a bad password, same message either way
  }
}
```

A duplicate signup is status 409. A password shorter than 8 characters is 400. Over the rate limit the status is 429.

## Data

`reactor.from(table)` and `reactor.rpc(name, args)` return the PostgREST client pointed at `/data/v1`. Filters, `order`, `limit`, embeds, and `rpc` are the PostgREST client’s API.

```ts
const { data, error } = await reactor
  .from("todos")
  .select("*")
  .eq("user_id", session.user.id)
  .order("created_at", { ascending: false })

const inserted = await reactor
  .from("todos")
  .insert({ title, user_id: session.user.id })
  .select()
```

A successful select looks like:

```json
{
  "data": [
    { "id": "…", "title": "Ship the site", "user_id": "6b1c1a4e-2f0a-4d3b-9c11-0a9e8d7c6b5a" }
  ],
  "error": null
}
```

An insert with `.select()` returns the inserted row in `data`, usually as an array. Row-level security still applies: a user token cannot read or write another user’s row, and the error comes back as `{ data: null, error }` rather than a thrown `ReactorError`. Auth, storage, and functions throw. Data does not.

## Storage and functions

```ts
await reactor.storage.from("files").upload(path, body, { contentType: "text/plain" })
const bytes = await reactor.storage.from("files").download(path)

const result = await reactor.functions.invoke("ping", { body: { hello: "world" } })
```

Upload asks `POST /storage/v1/object/presign` for a PUT URL, then sends the bytes to that URL. Download does the same with GET. Both methods resolve when the signed request succeeds. `upload` returns nothing. `download` returns a `Uint8Array`.

`invoke` is `POST /fn/v1/{name}` and returns the function’s JSON body. A ping that echoes its input looks like:

```json
{ "hello": "world" }
```

The exact shape is whatever the function writes to stdout. A non-zero exit throws `ReactorError` with status 500.

## Queue and enqueue

Create the client with the service key. `enqueue` is always available. `queue` needs [the extension](/product/extensions/).

```ts
const { id } = await reactor.functions.enqueue("ping", {
  body: { ok: true },
  delaySecs: 0,
  maxAttempts: 3,
})
const task = await reactor.functions.task(id)

await reactor.queue.create("jobs")
const { msg_id } = await reactor.queue.send("jobs", { hello: "world" })
await reactor.queue.subscribe("jobs", { functionName: "echo", vtSecs: 30, qty: 1, maxReads: 3 })
const rows = await reactor.queue.read("jobs", { vtSecs: 30, qty: 1 })
await reactor.queue.delete("jobs", rows[0].msg_id)
```

`task` returns `{ id, kind, status, attempts, max_attempts, last_error }`. `read` and `peek` return `{ msg_id, message, read_ct }[]`. `peek` does not hide the message.
