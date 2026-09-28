---
title: Kotlin
description: ReactorClient for Android and the JVM. Auth, a small query builder, storage, and functions.
---

The Kotlin client is published as `sl.atomicollabs.reactor:reactor-client:1.26.9-beta.1`. It is source-available under the Business Source License 1.1. The git tag is `v1.26.09-beta.1`.

```kotlin
val reactor = ReactorClient("https://<ref>.example.com", anonKey)
```

## Auth

```kotlin
val session = reactor.auth.signUp(email, password)
val session = reactor.auth.signInWithPassword(email, password)
val user = reactor.auth.getUser()
val session = reactor.auth.refreshSession()
reactor.auth.signOut()
```

`signInWithOAuth` throws. OAuth is not implemented.

Failures throw `ReactorException` with `status` and `message`.

## Data, storage, and functions

```kotlin
val inserted = reactor.from("todos")
  .insert(JSONObject().put("title", title).put("user_id", session.user.id))
  .select()
  .execute()

reactor.storage.from("files").upload(path, bytes)
val downloaded = reactor.storage.from("files").download(path)

val result = reactor.functions.invoke("ping")
```

The query builder covers the calls the first apps need. Use HTTP for the rest of PostgREST.
