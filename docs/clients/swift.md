---
title: Swift
description: ReactorClient for iOS 17 and macOS 14. Auth, queries, storage, and functions.
---

The Swift client is for iOS and macOS apps. It covers the same four surfaces as the JavaScript client: auth, a query builder for Postgres, file storage, and functions. The query builder is smaller than PostgREST. Use HTTP when you need embeds, `or`, or a range this builder does not express.

Add the package from GitHub. The release is the git tag `v1.26.10-beta8`. There is no second registry. Pin that tag. The product name is `Reactor`. It builds for iOS 17 and macOS 14.

```swift
.package(url: "https://github.com/reactor-cloud/reactor-swift", exact: "1.26.10-beta8")
```

```swift
import Reactor

let reactor = ReactorClient(
  url: "https://<ref>.example.com",
  anonKey: anonKey,
  sessionStore: KeychainSessionStore()
)
```

`sessionStore` defaults to memory, which lasts until the process exits. `KeychainSessionStore` keeps the session under the service name `lab.reactor.session`.

## Auth

```swift
let outcome = try await reactor.auth.signUp(email: email, password: password)
let outcome = try await reactor.auth.signInWithPassword(email: email, password: password)
let user = try await reactor.auth.getUser()
let session = try await reactor.auth.refreshSession()
try await reactor.auth.signOut()
let session = reactor.auth.getSession()
```

JSON from the server uses `access_token` and `refresh_token`. The Swift properties are `accessToken` and `refreshToken`. A session from signup or password sign-in is:

```json
{
  "access_token": "eyJ...",
  "refresh_token": "opaque-token",
  "user": { "id": "6b1c1a4e-2f0a-4d3b-9c11-0a9e8d7c6b5a", "email": "ada@example.com" }
}
```

`signUp` and `signInWithPassword` return `AuthOutcome`: `.session`, `.verificationRequired`, `.mfaRequired`, or `.enrollmentRequired`. Only `.session` is stored. `verifyEmail`, `verifyTotp`, `verifyPasskey`, `verifyRecovery`, `enrollTotp`, and `enrollPasskey` call the matching routes. `signInWithOAuth(provider:redirectTo:)` returns the authorize URL. `exchangeCode` stores the session.

`getUser()` returns the id and email. `getSession()` returns the stored session, or nil when nobody is signed in.

Failures throw `ReactorError` with `status` and `message`. A bad password is status 401. The message does not say whether the email exists.

## Data

```swift
let rows = try await reactor.from("todos")
  .select()
  .eq("user_id", session.user.id)
  .order("created_at", ascending: false)
  .limit(50)
  .execute()

let created = try await reactor.from("todos")
  .insert(["title": .string(title), "user_id": .string(session.user.id)])
  .execute()
```

`execute()` returns the JSON rows. A select of todos is an array of objects with `id`, `title`, and `user_id`. Writes send `Prefer: return=representation`, so `insert` returns the stored row rather than an empty body. `update`, `delete`, and `eq` chain the same way.

## Storage and functions

```swift
try await reactor.storage.from("files").upload(path: path, data: data, contentType: "text/plain")
let data = try await reactor.storage.from("files").download(path: path)

let result = try await reactor.functions.invoke("ping", body: .object(["hello": .string("world")]))
```

Upload and download presign, then call the signed URL. `upload` finishes when the PUT succeeds. `download` returns the bytes. `invoke` returns the function’s JSON. A ping that echoes its body comes back as an object with `hello` set to `world`.

## Queue and enqueue

Construct the client with the service key. `enqueue` is always available. `queue` needs [the extension](/product/extensions/).

```swift
let task = try await reactor.functions.enqueue(
  "ping",
  body: .object(["ok": .bool(true)]),
  delaySecs: 0,
  maxAttempts: 3
)
let status = try await reactor.functions.task(task["id"]?.string() ?? "")

try await reactor.queue.create("jobs")
let sent = try await reactor.queue.send("jobs", message: .object(["hello": .string("world")]))
try await reactor.queue.subscribe("jobs", functionName: "echo", vtSecs: 30, qty: 1, maxReads: 3)
```

`task` is `{ id, kind, status, attempts, max_attempts, last_error }`. `read` and `peek` return message rows. `peek` does not hide the message.
