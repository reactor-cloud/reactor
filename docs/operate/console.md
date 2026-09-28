---
title: Console
description: The cluster admin UI. Operators are not project users.
---

The console is served by the Reactor binary at `/console`. The API is `/console/v1`. It is not on a project hostname, and it is not a public signup for reactor.cloud.

`v2/console` is a Vite app. The image carries the built files. Locally that is `http://127.0.0.1:18000/console`.

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

An operator can enroll a TOTP authenticator or a passkey. When one is enrolled, login finishes through `/console/v1/mfa`.

## Pages

Outside a project the sidebar lists Projects and, for a platform admin, Cluster and Console users. Inside a project it lists Overview, API keys, Auth, Data, Storage, Functions, and Sites, with Logs, Team, and Settings at the bottom.

- **Overview** shows the project name, counts, and 24-hour traffic for auth, database, functions, and sites.
- **API keys** shows the anon key and a hidden service key, with reveal, copy, and rotate. Plaintext exists only at create and rotate.
- **Auth** lists project users. An owner or admin can change a password or delete a user.
- **Data** lists tables, can create one, and opens a grid. That grid is every row in the schema. It does not apply a user's row-level security policy.
- **Storage** is the blob browser under the project prefix.
- **Functions** opens Overview, Versions, Logs, Variables, and Test. Promote pins a version. Demote clears the pin. Variables are encrypted per function and the value is never shown again.
- **Sites** opens the project's site: Overview, Deployments, Logs, Variables, and Domains. A custom host shows the TXT record until it verifies.
- **Logs** are recent function and site lines.
- **Settings** renames or deletes the project. Delete asks for the project name.

The cluster pill shows the name, where it runs (local Docker, AWS, or Fly), and a health dot.

## Agent

To the right of the avatar, Reactor Operator opens a conversation sidebar for that operator. Turns are stored and polled while they run. The agent can see the project the operator has open. It is not a public API on reactor.cloud.
