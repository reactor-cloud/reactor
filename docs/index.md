---
title: Introduction
description: Auth, Postgres data, file storage, functions, and sites from one Rust server.
---

Reactor is a Rust backend for web and mobile apps. One server gives you auth, Postgres data through PostgREST, file storage, functions, and sites. Deploy it on AWS or Fly, or run it locally with Docker. Postgres and a blob store are the only dependencies.

A project you create on your machine uses the same client URLs it will use on any other cluster. The JavaScript client follows the same shape as the Supabase client, so an app written against `createClient`, `auth`, and `from()` can move with small changes.

**v1.26.09-beta.3.** Package version `1.26.9-beta.3`. The API can still change before a stable tag.

## Install

The CLI on a Mac:

```sh
brew tap reactor-cloud/reactor
brew install reactor
```

Apple silicon macOS 26 downloads a bottle. Other Macs compile from source and need Rust. [CLI](/operate/cli/) also covers installing from a clone.

The JavaScript client:

```sh
npm install @reactor-cloud/client@beta
```

Swift and Kotlin are documented under [Clients](/clients/javascript/).

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

The [todos example](/examples/todos/) is a static site, a table with row-level security, and one function.

## Not included

Realtime, analytics, and billing are not part of this release. There is no SQL editor and no second data API. Console MFA is for operators. Project users can verify email, enroll a second factor, and sign in with a configured OAuth provider. Hosted Reactor is not open yet. [Self-host](/operate/self-hosting/) the server, or follow the [quickstart](/start/quickstart/).

## License

You can use Reactor as the backend for as many personal or commercial projects as you want, including setting it up for a client who runs it. The license only restricts offering Reactor’s auth, data, storage, functions, or sites to other people as a hosted service. This version becomes Apache 2.0 on 2030-09-28. See the repository `LICENSE`.
