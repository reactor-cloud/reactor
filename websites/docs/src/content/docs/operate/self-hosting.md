---
title: Self-hosting
description: Compose for a local cluster, and the same image on Fly.
---

The local path is Docker Compose. The image is the same one you run elsewhere.

## Compose

From the repository root:

```sh
docker compose -f deploy/compose/compose.yaml build keys
docker compose -f deploy/compose/compose.yaml up -d
```

| Port | Process |
| --- | --- |
| `127.0.0.1:18000` | Reactor. Console at `/console`. Health at `/health` |
| `127.0.0.1:18001` | A second Reactor process, same database and bucket |
| `127.0.0.1:5440` | Shared Postgres |
| `127.0.0.1:5441` | Dedicated-database Postgres |
| `127.0.0.1:19000` | S3-compatible blob store |

The `keys` service writes the JWT private key and exits. Both app containers mount that volume read-only. Function workdirs are separate volumes, so a zip unpacked on one replica is not assumed to exist on the other. The blob store is the source of truth. Invoke unpacks again when the local copy is missing.

`base_domain` is `apps.localhost`. A project site is `http://{ref}.apps.localhost:18000/`.

First-run setup is the console or `reactor setup`. Then [create a project](/start/quickstart/).

## Fly

`v2/deploy/fly/fly.toml` runs the same image in `listen` mode. `place` is `fly`, so the console shows that. PostgREST is on `127.0.0.1:3000` inside the machine. Blobs are S3-compatible storage, not the machine disk. The health check is `GET /health`.

Secrets (database URL, operator token, storage keys, JWT) stay in the Fly secret store. They are not in the toml file that is committed.

## More than one process

Add replicas when CPU is the limit. They share Postgres and the blob bucket. Nothing in the client URL changes. Add a dedicated database when one project is the limit, by setting that project's `database_url`. Callers still use the same host and the same keys.

The AWS layout is the [Lambda](/operate/lambda/) page.

## Backup and restore

Dump the control database, including every project schema, and copy the blob bucket. Both are required. A dump without the bucket loses function zips and site files. A bucket without the dump loses users, keys, and row data.

```sh
pg_dump --no-owner --no-acl "$DATABASE_URL" > reactor.sql
```

Restore by loading that file into an empty Postgres, then copying the bucket back to the same key layout. `deploy/fly/backup` is the Fly job that writes the dump into object storage. It is one way to run the dump, not the only one.
