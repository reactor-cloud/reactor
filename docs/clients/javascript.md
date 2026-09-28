---
title: JavaScript
description: createClient for auth, PostgREST data, storage, and functions.
---

The JavaScript client is published as `@reactor/client` at `1.26.9-beta.1` under BUSL-1.1. The prerelease dist-tag is `beta`.

```sh
npm install @reactor/client@beta
```

It needs `@supabase/postgrest-js`, which is how data queries are built.

```ts
import { createClient } from "@reactor/client"

const reactor = createClient("https://<ref>.apps.localhost:18000", anonKey, {
  session: savedSession, // optional
})
```

The first argument is the project origin with no trailing slash. The second is the anon key. Options are `fetch` and an initial `session`.

## Auth

```ts
const session = await reactor.auth.signUp({ email, password })
const session = await reactor.auth.signInWithPassword({ email, password })
const user = await reactor.auth.getUser()
const session = await reactor.auth.refreshSession()
await reactor.auth.signOut()
reactor.auth.getSession()
```

A session is `{ access_token, refresh_token, user: { id, email } }`. The client keeps it in memory. Persist it yourself if the page should survive a reload. Later calls send the access token. Before sign-in they send the anon key.

`signInWithOAuth` throws. OAuth is not implemented.

## Data

`reactor.from(table)` and `reactor.rpc(name, args)` return the PostgREST client pointed at `/data/v1`.

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

Errors from PostgREST come back as `{ error }` on the query result. Auth and storage throw `ReactorError` with `status` and `message`.

## Storage and functions

```ts
await reactor.storage.from("files").upload(path, body, { contentType: "text/plain" })
const bytes = await reactor.storage.from("files").download(path)

const result = await reactor.functions.invoke("ping", { body: { hello: "world" } })
```

Upload and download presign, then call the signed URL. Invoke is `POST /fn/v1/{name}` and returns the JSON body.
