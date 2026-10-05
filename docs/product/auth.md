---
title: Auth
description: Project users, email verification, second factors, and OAuth.
---

Auth is how a project gets users. It is not the console login. A person who signs up here can read and write rows their policies allow. They cannot open `/console`. Routes live at `/auth/v1`. A request must resolve a project from the host or from a bearer token. Signup and password login accept the anon key.

The JavaScript client’s `auth.signUp` and `auth.signInWithPassword` call these routes and return the session below. See [Concepts](/start/concepts/) for how that token differs from an operator token.

## Password

`POST /auth/v1/signup`

```json
{ "email": "ada@example.com", "password": "at-least-8" }
```

Password shorter than 8 characters is 400. A duplicate email is 409. Email is stored lowercased and trimmed.

`POST /auth/v1/token` with email and password signs in. Unknown email or a bad password is 401, with the same error text, so the response does not reveal which addresses exist:

```json
{ "error": "invalid credentials" }
```

New projects require email verification. Signup then returns 200 and no tokens:

```json
{ "verification_required": true, "user": { "id": "...", "email": "ada@example.com" } }
```

The message contains a link and a 6-digit code. `POST /auth/v1/verify-email` with `{ "token" }` or `{ "email", "code" }` returns a session and sets `email_verified_at`. Either value works once. Five wrong codes burn the challenge. `POST /auth/v1/verify-email/send` with `{ "email" }` replaces the previous challenge. A second send inside 60 seconds does not send mail. The response is always `{ "ok": true }`.

Password login of an unverified user returns the same `verification_required` body and does not send another message. Turn verification off in the console and signup and password login return a session with no confirm mail.

When 2FA is required, a verified password login with no factor returns `{ "enrollment_required": true, "enroll_token": "...", "factors": [] }`. That token can only enroll. Finish enroll returns eight backup codes once and a session. When a factor is already enrolled the body is `{ "mfa_required": true, "mfa_token": "...", "factors": ["totp"] }`. `POST /auth/v1/factors/totp`, `/auth/v1/factors/passkey/verify`, and `/auth/v1/factors/recovery` return a session. Five failures burn the challenge. With 2FA off, password login returns a session even if factors exist. OAuth is not challenged. Signup with verification off returns a session even when 2FA is on.

A session, when one is issued:

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

`GET /auth/v1/user` with the user access token returns `{ "id", "email", "email_verified_at" }`. An anon or service token is 401.

## Email flows

Magic links, recovery, invites, and email confirmation send mail through SMTP. The project server wins when its host is set. Otherwise, if a platform admin has allowed the project to use the cluster server and that host is set, mail uses the cluster server. If neither host is set, there is no mail. The From address follows the server that sends. The link base stays on the project even when the cluster server sends.

Each server stores host, port (default 587), username, password, From address, and TLS: `starttls`, `tls`, or `none`. The password is write-only. A later read shows `password_set` and never the secret. Configure the project under Auth → Email. A platform admin stores the cluster server on the Cluster page and turns `cluster_smtp` on per project. Templates are `confirm_email`, `magic_link`, `recovery`, `invite`, and `otp`. `confirm_email` includes `{{code}}` and `{{link}}`. `otp` includes `{{code}}`. You can replace a template; reset restores the built-in one.

`POST /auth/v1/magic-link` with `{ "email" }` and the anon key. The response is always:

```json
{ "ok": true }
```

A new email creates a user. The link token lasts 15 minutes. A second request for the same email within 60 seconds does not send another message.

`POST /auth/v1/verify` with `{ "token" }` consumes a magic-link token, sets `email_verified_at`, and returns a session.

### Email code

`POST /auth/v1/otp` with `{ "email" }` and the anon key mails a 6-digit code that lasts 10 minutes. A malformed address is 400. Otherwise the response is `{ "ok": true }` for new and existing addresses alike, and never contains the code. No user is created on send. A new code replaces the previous one. A second request within 60 seconds does not send another message.

`POST /auth/v1/otp/verify` with `{ "email", "code" }` returns a session. It creates a user with no password when the email is new, and returns the existing user otherwise. It sets `email_verified_at` when it is empty and never changes a password. A wrong, expired, or used code is 400 with `{ "error": "invalid or expired code" }`. Five wrong guesses burn the code.

Caps return 429 `{ "error": "too many requests" }`:

- 5 codes per email per hour, 10 per day.
- 100 codes per project per hour.
- 10 wrong guesses per email per hour. Verify is refused until the hour passes, even with a fresh code.
- 20 sends and 30 verifies per client IP per minute.

`POST /auth/v1/recover` with `{ "email" }` is also always `{ "ok": true }`, including when the email is unknown. That avoids confirming which addresses exist.

`POST /auth/v1/recover/complete` with `{ "token", "password" }` sets the password, marks the email verified, and returns a session.

`POST /auth/v1/invite` requires the service key and `{ "email" }`. If email is not configured the response is 503. If the message cannot be sent it is 502 and the user row is removed. An existing email is 409. The invite lasts 7 days.

`POST /auth/v1/invite/accept` with `{ "token", "password" }` sets the password, marks the email verified, and returns a session.

If SMTP is missing, a signup that requires verification is 503 and the new user row is removed. A send failure is 502 and the row is removed.

## Second factor

Passkey is a second factor, not a passwordless login. Enroll while signed in with the user access token, or with the `enroll_token` from password login:

- `POST /auth/v1/factors/totp/start` then `POST /auth/v1/factors/totp/confirm` with `{ "code" }`
- `POST /auth/v1/factors/passkey/register/options` then `POST /auth/v1/factors/passkey/register`
- `POST /auth/v1/factors/finish` after at least one factor is enrolled
- `POST /auth/v1/factors/recovery/regenerate` with the user access token

A password challenge uses `POST /auth/v1/factors/totp` with `{ "mfa_token", "code" }`, `POST /auth/v1/factors/passkey/options` then `/auth/v1/factors/passkey/verify`, or `POST /auth/v1/factors/recovery` with a backup code.

## OAuth

The project owner pastes credentials for Google, Microsoft, Apple, GitHub, Facebook, Discord, X, LinkedIn, Slack, or GitLab. Microsoft needs `extra.tenant` (use `common` for any work or personal account). Apple needs `extra.team_id` and a client secret the server signs as a JWT. Each provider has a redirect allowlist. `GET /auth/v1/authorize?provider=&redirect_to=` checks that list, stores PKCE for 10 minutes, and redirects to the provider. The callback is `{origin}/auth/v1/callback/{provider}`. Register that URL with the provider. Apple posts the callback; the others use GET.

A known provider subject signs that user in. A verified email that matches a user links the identity. An unverified email does not link and does not create a user; the browser is sent to `redirect_to?error=email_not_verified`. A new verified email creates a user with no password. The browser is then sent to `redirect_to?code=`. `POST /auth/v1/token` with `{ "code" }` returns a session. Reusing the code is 401. A redirect that is not on the allowlist is 400. A disabled provider is 404.

Every finished path returns the same session. Data and storage do not learn the method.

Console operator login is separate and is not these routes.

Signing keys live on the server (`REACTOR_AUTH__JWT_DIR` or a PEM file). Clients never see the private key. PostgREST verifies the same public key.
