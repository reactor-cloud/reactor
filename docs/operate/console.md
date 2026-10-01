---
title: Console
description: The cluster admin UI. Operators are not project users.
---

The console is the admin UI for a cluster. Operators sign in here to create projects, inspect data and files, and ship functions and sites. Project users never use it. Their signup lives on the project host.

The server binary serves the built UI at `/console`. The API is `/console/v1`. It is not on a project hostname, and it is not a public signup for reactor.cloud. The UI source is the `console/` directory. The image carries the built files. Locally that is `http://127.0.0.1:18000/console`.

## Operators

The first setup call creates the cluster name and the first operator with `platform_admin`. After that, people are invited onto a project by an owner or admin. A new email gets an operator row and a password set at invite time.

An operator can belong to many projects and still have no row in `reactor.users`. Membership roles are `owner`, `admin`, and `developer`. Creating a project makes the caller the owner.

| Action | Minimum role |
| --- | --- |
| Cluster page, console users | platform admin |
| Create a project | any operator |
| Data, storage, functions, sites, logs | developer |
| Project users, team, rotate API keys | admin |
| Rename or delete the project | owner |

Console JWTs use the same Ed25519 key with `aud=console` and `role=console`. They are rejected by `/auth/v1` and `/data/v1`. Project JWTs are rejected by `/console/v1`.

A console service key uses `aud=console-key`. A human session mints it from the avatar menu. The secret is shown once. `projects.create` can create projects. `auth.settings`, `auth.providers`, `auth.email`, and `auth.users` work only on projects that same key created. Revoke deletes the key and clears that stamp.

An operator can enroll a TOTP authenticator or a passkey. When one is enrolled, login finishes through `/console/v1/mfa`.

## Pages

Outside a project the sidebar lists Projects and, for a platform admin, Cluster and Console users. Service keys is in the avatar menu for every operator. Inside a project it lists Overview, API keys, Auth, Data, Storage, Functions, and Sites, with Logs, Team, and Settings at the bottom.

- **Overview** shows the project name, counts, and 24-hour traffic for auth, database, functions, and sites.
- **API keys** shows the anon key and a hidden service key, with reveal, copy, and rotate. Plaintext exists only at create and rotate.
- **Auth** has an inner sidebar: Users, Providers, 2FA, Email, and Templates. Users lists project users and whether the email is verified. An owner or admin can change a password or delete a user. Providers turns email verification on or off and stores OAuth client credentials. 2FA turns on a second factor for password sign-in. Email is the project's SMTP server and link base. A platform admin can allow that project to use the cluster SMTP server, which sends when the project host is empty. Templates edits `confirm_email`, `magic_link`, `recovery`, and `invite`. `/email` redirects to Auth → Email.
- **Data** lists tables, can create one, and opens a grid. That grid is every row in the schema. It does not apply a user's row-level security policy.
- **Storage** is the blob browser under the project prefix.
- **Functions** opens Overview, Versions, Logs, Variables, and Test. Promote pins a version. Demote clears the pin. Variables are encrypted per function and the value is never shown again.
- **Sites** opens the project's site: Overview, Deployments, Logs, Variables, and Domains. A custom host shows the TXT record until it verifies.
- **Logs** are recent function and site lines.
- **Settings** renames or deletes the project. Delete asks for the project name.

The cluster pill shows the name, where it runs (local Docker, AWS, or Fly), and a health dot. The Cluster page stores the cluster SMTP server. A project uses it for auth mail only after a platform admin allows that project, and only when the project has no SMTP host of its own. Creating a project in the UI returns the same pair the CLI prints once: `anon_key` and `service_key`. Later visits show the anon key and a hidden service key. Reveal does not mint a new secret. Rotate does, and the previous keys stop working.

A console call with a project user token is rejected. A project call with a console token is rejected. Login with MFA enrolled returns a short-lived token that only `/console/v1/mfa` accepts until the second factor succeeds.

## Agent

To the right of the avatar, Reactor Operator opens a conversation sidebar for that operator. Turns are stored and polled while they run. The agent can see the project the operator has open. It is not a public API on reactor.cloud.
