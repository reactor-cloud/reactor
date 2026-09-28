---
title: Swift
description: ReactorClient for iOS and macOS. Auth, a small query builder, storage, and functions.
---

The Swift package is published from tag `v1.26.09-beta.1` at `https://github.com/reactor-cloud/reactor-swift`. It builds for iOS 17 and macOS 14. The product name is `Reactor`. It is source-available under BUSL-1.1, version `1.26.9-beta.1`.

```swift
import Reactor

let reactor = ReactorClient(
  url: "https://<ref>.example.com",
  anonKey: anonKey,
  sessionStore: KeychainSessionStore()
)
```

`sessionStore` defaults to memory. `KeychainSessionStore` keeps the session under the service name `lab.reactor.session`.

## Auth

```swift
let session = try await reactor.auth.signUp(email: email, password: password)
let session = try await reactor.auth.signInWithPassword(email: email, password: password)
let user = try await reactor.auth.getUser()
let session = try await reactor.auth.refreshSession()
try await reactor.auth.signOut()
let session = reactor.auth.getSession()
```

JSON from the server uses `access_token` and `refresh_token`. The Swift properties are `accessToken` and `refreshToken`.

`signInWithOAuth` throws. OAuth is not implemented.

Failures throw `ReactorError` with `status` and `message`.

## Data

The query builder covers the calls the first apps need. It is not the full PostgREST surface. Use HTTP for embeds, `or`, and range that this builder does not express.

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

`update`, `delete`, and `eq` chain the same way. Writes send `Prefer: return=representation`.

## Storage and functions

```swift
try await reactor.storage.from("files").upload(path: path, data: data, contentType: "text/plain")
let data = try await reactor.storage.from("files").download(path: path)

let result = try await reactor.functions.invoke("ping", body: .object(["hello": .string("world")]))
```
