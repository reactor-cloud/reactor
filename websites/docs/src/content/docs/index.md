---
title: Introduction
description: Auth, data, storage, functions, and sites on one source-available server.
---

Reactor is a source-available backend for web and mobile apps. One stateless server serves auth, Postgres data through PostgREST, file storage, functions, and sites. Postgres and a blob store are the only dependencies.

**v1.26.09-beta.1** (package version `1.26.9-beta.1`). The API can still change before a stable tag. You may run Reactor for your own product, including a commercial one, and you may set it up for a client who operates it. You may not offer Reactor’s auth, data API, storage, functions, or sites to third parties as a hosted service without a commercial license. On 2030-09-28 this version becomes Apache 2.0. See the repository `LICENSE`.

The same image runs as a long-lived process or as Lambda handlers. A project created on your machine uses the same client URLs it will use anywhere else.

Hosted Reactor is not open yet. [Self-host](/operate/self-hosting/) the server, or read the [quickstart](/start/quickstart/) and run the local Compose stack.

## What you get

| Surface | Path | What it is |
| --- | --- | --- |
| Auth | `/auth/v1` | Users, sessions, magic links, recovery, invites |
| Data | `/data/v1` | PostgREST, with row-level security |
| Storage | `/storage/v1` | Presigned uploads and downloads |
| Functions | `/fn/v1` | Bun or Lambda, with versions and pins |
| Sites | project host | Static files, a site process, or a function path |
| Console | `/console` | Cluster admin. Separate from project users |

## Start here

1. [Concepts](/start/concepts/) — tenancy, tokens, and the data boundary.
2. [Quickstart](/start/quickstart/) — Compose, the console, a project, a deploy.
3. [Projects](/start/projects/) — refs, schemas, keys, and hostnames.

JavaScript, Swift, and Kotlin clients live under [Clients](/clients/javascript/). The [todos example](/examples/todos/) is a static site, a table with row-level security, and one function.

## Not included

Realtime, OAuth, project-user MFA, analytics, and billing are not part of this release. Console MFA is for operators. There is no SQL editor and no second data API.
