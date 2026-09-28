---
title: Auth
description: Project users, sessions, magic links, recovery, and invites.
---

Auth is per project. Routes live at `/auth/v1`. A request must resolve a project from the host or from a bearer token. Signup and password login accept the anon key.

Project users are not console operators. See [Concepts](/start/concepts/).

## Password

`POST /auth/v1/signup`

```json
{ "email": "ada@example.com", "password": "at-least-8" }
```

Password shorter than 8 characters is 400. A duplicate email is 409. Email is stored lowercased and trimmed.

`POST /auth/v1/token` with email and password signs in. Unknown email or a bad password is 401, with the same error text.

Both return:

```json
{
  "access_token": "...",
  "refresh_token": "...",
  "user": { "id": "...", "email": "ada@example.com" }
}
```

The access token is an Ed25519 JWT. It expires in 15 minutes. Claims include the project ref, the user id as `sub`, and role `authenticated`.

`POST /auth/v1/token` with `{ "refresh_token": "..." }` rotates the refresh token and returns a new pair. The old refresh row is deleted in the same transaction. An unknown or expired refresh token is 401. Refresh tokens last 30 days.

`POST /auth/v1/logout` with `{ "refresh_token": "..." }` deletes that session and returns 204.

`GET /auth/v1/user` with the user access token returns `{ "id", "email" }`. An anon or service token is 401.

## Email flows

Magic links, recovery, and invites send mail through the project's SMTP settings. Configure those in the console under the project email settings. Templates are `magic_link`, `recovery`, and `invite`. You can replace a template; reset restores the built-in one.

`POST /auth/v1/magic-link` with `{ "email" }` and the anon key. The response is always `{ "ok": true }`. A new email creates a user. The link token lasts 15 minutes. A second request for the same email within 60 seconds does not send another message.

`POST /auth/v1/verify` with `{ "token" }` consumes a magic-link token and returns a session.

`POST /auth/v1/recover` with `{ "email" }` is also always `{ "ok": true }`, including when the email is unknown. That avoids confirming which addresses exist.

`POST /auth/v1/recover/complete` with `{ "token", "password" }` sets the password and returns a session.

`POST /auth/v1/invite` requires the service key and `{ "email" }`. If email is not configured the response is 503. If the message cannot be sent it is 502 and the user row is removed. An existing email is 409. The invite lasts 7 days.

`POST /auth/v1/invite/accept` with `{ "token", "password" }` sets the password and returns a session.

## What is not here

OAuth and external identity providers are not implemented. The JavaScript, Swift, and Kotlin clients throw if you call `signInWithOAuth`. A later provider has to return the same identity: project, user id, role, and claims. Auth, data, storage, functions, and sites do not learn which issuer it was.

Project-user MFA is not a route on `/auth/v1`. Console MFA applies to operators only.

Signing keys live on the server (`REACTOR_AUTH__JWT_DIR` or a PEM file). Clients never see the private key. PostgREST verifies the same public key.
