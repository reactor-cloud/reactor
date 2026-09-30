# Changelog

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
