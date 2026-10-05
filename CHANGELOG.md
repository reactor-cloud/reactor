# Changelog

## v1.26.10-beta7

Package manifests use `1.26.10-beta7`. The JavaScript, Swift, and Kotlin clients stay on `1.26.9-beta.2`.

- The console SQL page and `reactor sql` run as a per-project database role. Reads are the default. Destructive statements ask for confirmation. A migration can be stored and reverted.
- A console service key with `projects.sql` can run that SQL on projects the key created. Each key token has its own `jti`.
- `auth.email` can allow cluster SMTP on projects that key created when the operator is a platform admin. The cluster mail server stays closed.

## v1.26.10-beta6

Package manifests use `1.26.10-beta6`. The JavaScript, Swift, and Kotlin clients stay on `1.26.9-beta.2`.

- `POST /auth/v1/otp` mails a 6-digit sign-in code. `POST /auth/v1/otp/verify` returns a session. The code lasts 10 minutes and is rate limited.

## v1.26.10-beta5

Package manifests use `1.26.10-beta5`. The JavaScript, Swift, and Kotlin clients stay on `1.26.9-beta.2`.

- The console data page lists a project's own schemas and edits a row in the selected schema.
- Domain verify reads the TXT record over HTTPS when `REACTOR_DNS_STUB_FILE` is unset.

## v1.26.09-beta.4

Package manifests use `1.26.9-beta.4`. OpenAPI uses `1.26.09-beta.4`. The JavaScript, Swift, and Kotlin clients stay on `1.26.9-beta.2`.

- Console service keys can apply one SQL file with `projects.migrate`, only on projects that key created.
- `reactor service-keys` lists, creates, and revokes those keys.

## v1.26.09-beta.3

Package manifests use `1.26.9-beta.3`. OpenAPI uses `1.26.09-beta.3`. The JavaScript, Swift, and Kotlin clients stay on `1.26.9-beta.2`.

- An operator can mint a console service key from the avatar menu. The secret is shown once. The token audience is `console-key`.
- Scopes are `projects.create`, `auth.settings`, `auth.providers`, `auth.email`, and `auth.users`. Auth scopes work only on projects that same key created. Revoke deletes the key.
- PostgREST still reloads its schema list when Aurora rejects `ALTER ROLE`, so a project created after boot is visible to `/data`.

## v1.26.09-beta.2

Package manifests use `1.26.9-beta.2`. OpenAPI and the Swift pin use `1.26.09-beta.2`.

- New projects require email verification. Signup and password login return a session, a verification body, or a 2FA challenge.
- Project users can enroll TOTP, a passkey, and backup codes. Operator MFA is unchanged.
- Customer-supplied OAuth for Google, Microsoft, Apple, GitHub, Facebook, Discord, X, LinkedIn, Slack, and GitLab.
- The console Auth page has Users, Providers, 2FA, Email, and Templates.

## v1.26.09-beta.1

First public beta. Package manifests use `1.26.9-beta.1` because semver rejects a leading zero in `09`.

- Auth, PostgREST data with row-level security, storage, functions, sites, and the console in one image
- JavaScript, Swift, and Kotlin clients
- Business Source License 1.1, changing to Apache 2.0 on 2030-09-28
