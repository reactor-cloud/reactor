---
title: Extensions
description: Optional products turned on for the whole cluster, starting with Queue.
---

An extension is a product the server can run without it being part of every cluster. Auth, data, storage, functions, and sites are always on. Queue is not. Set `REACTOR_EXTENSIONS` to the names you want, separated by commas. An unknown name refuses to start.

```sh
REACTOR_EXTENSIONS=queue
```

The name is the whole contract. There is no per-project switch. A cluster either has the routes or it does not.

## What is on

The operator token can list them:

```http
GET /platform/v1/extensions
```

```json
[{ "name": "queue", "routes": ["POST /queue/v1/queues", "GET /queue/v1/queues"] }]
```

The console uses `GET /console/v1/extensions` with a console session and shows a nav item only when that name is present. An empty list means the variable was empty.

## Project SQL

An extension can ship SQL that belongs in each project schema. Reactor applies it when a project is created or migrated, after the files in `sql/project/`. Queue's file is recorded as `x_queue/001_queue.sql`. Those tables stay in `proj_<ref>`. Row-level security is forced, and the grants are for the `service` role.

Turn the extension on before you create the project, or run migrate after you turn it on. A project created while the extension was off does not have the tables until the next migrate.

## When work runs

Listen mode runs a tick about once a second. That tick claims due tasks, then runs hooks that are due. Queue's hook is one of those. It drains subscriptions.

Lambda does not start that loop. Call the tick yourself with the operator token:

```http
POST /_internal/tick
```

`?force=1` ignores how long it has been since a hook last ran. It does not run a task whose `run_at` is still in the future.

Done tasks are deleted after 24 hours. Sessions older than their expiry are deleted on the same clock.

## Tasks and queues

Enqueueing a function is core. It does not require an extension. `POST /fn/v1/{name}/enqueue` with the service key stores a task. The tick runs it, retries it, and writes a `function` log when the function actually runs. That API is documented on [Functions](/product/functions/).

A queue is the extension. It stores messages and, when you subscribe, asks a function to handle each one. That API is documented on [Queue](/product/queue/).

Both show up in the console logs. A function run is kind `function`. A queue delivery is kind `queue`, named `{queue}:{function}`. The overview chart adds a series for any kind that has traffic in the last 24 hours.
