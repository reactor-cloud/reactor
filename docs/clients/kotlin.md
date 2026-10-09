---
title: Kotlin
description: ReactorClient for Android and the JVM. Auth, queries, storage, and functions.
---

The Kotlin client is for Android and other JVM apps. It covers auth, a small query builder, file storage, and functions. The query builder is enough for the first screens of an app. Use HTTP for the rest of PostgREST.

The coordinates are `sl.atomicollabs.reactor:reactor-client:1.26.10-beta9`. The git tag is `v1.26.10-beta9` on [reactor-cloud/reactor-kotlin](https://github.com/reactor-cloud/reactor-kotlin). Publishing to Maven Central still needs that portal’s token and a GPG key. Until those are in place, build the module from the repository.

```kotlin
dependencies {
    implementation("sl.atomicollabs.reactor:reactor-client:1.26.10-beta9")
}
```

```kotlin
val reactor = ReactorClient("https://<ref>.example.com", anonKey)
```

The first argument is the project origin. The second is the anon key. The client uses that key until a session exists, then sends the access token.

## Auth

```kotlin
val result = reactor.auth.signUp(email, password)
val result = reactor.auth.signInWithPassword(email, password)
val user = reactor.auth.getUser()
val session = reactor.auth.refreshSession()
reactor.auth.signOut()
```

Signup and password sign-in return:

```json
{
  "access_token": "eyJ...",
  "refresh_token": "opaque-token",
  "user": { "id": "6b1c1a4e-2f0a-4d3b-9c11-0a9e8d7c6b5a", "email": "ada@example.com" }
}
```

`signUp` and `signInWithPassword` return `AuthResult.SignedIn`, `VerificationRequired`, `MfaRequired`, or `EnrollmentRequired`. Only `SignedIn` is stored. `verifyEmail`, `verifyTotp`, `verifyPasskey`, `verifyRecovery`, `enrollTotp`, and `enrollPasskey` call the matching routes. `signInWithOAuth(provider, redirectTo)` returns the authorize URL. `exchangeCode` stores the session. Magic link, recovery, and invite are not on this client.

`getUser()` returns id and email. `refreshSession()` replaces the stored pair. `signOut()` deletes the refresh token on the server.

Failures throw `ReactorException` with `status` and `message`. Status 401 is a bad password or an unknown email, with the same message either way. Status 429 means the project has hit the auth rate limit.

## Data, storage, and functions

```kotlin
val inserted = reactor.from("todos")
    .insert(JSONObject().put("title", title).put("user_id", session.user.id))
    .select()
    .execute()

reactor.storage.from("files").upload(path, bytes)
val downloaded = reactor.storage.from("files").download(path)
reactor.storage.createBucket("photos", public = true)
val url = reactor.storage.from("photos").getPublicUrl(path)

val result = reactor.functions.invoke("ping")
```

`execute()` returns the JSON from PostgREST. `.select()` after insert asks for the stored row, so `inserted` is that row rather than an empty body. `upload` presigns a PUT and sends the bytes. `download` returns the file bytes. `invoke` without a body posts `{}` and returns the function’s JSON. A ping with no extra fields still returns whatever `index.ts` writes to stdout.

## Queue and enqueue

Construct the client with the service key. `enqueue` is always available. `queue` needs [the extension](/product/extensions/).

```kotlin
val task = reactor.functions.enqueue("ping", JSONObject().put("ok", true), delaySecs = 0, maxAttempts = 3)
val status = reactor.functions.task(task.getString("id"))

reactor.queue.create("jobs")
val sent = reactor.queue.send("jobs", JSONObject().put("hello", "world"))
reactor.queue.subscribe("jobs", "echo", vtSecs = 30, qty = 1, maxReads = 3)
```

`task` is `{ id, kind, status, attempts, max_attempts, last_error }`. `read` and `peek` return a `JSONArray` of message rows. `peek` does not hide the message.
