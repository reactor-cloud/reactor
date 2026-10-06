---
title: Queue
description: Messages in a project, delivered to functions when you subscribe.
---

A queue belongs to one project. It is off until the cluster sets `REACTOR_EXTENSIONS=queue`. See [Extensions](/product/extensions/) for how that switch works and when the drain runs.

Service keys send and read messages. Anon and user tokens are refused. One project's schema cannot see another's rows. Names match `^[a-z][a-z0-9_]{0,62}$`.

A subscription is separate from [function enqueue](/product/functions/#enqueue). Enqueue runs one function later. A subscription delivers whatever is sitting in a queue.

## Messages

`send` appends a JSON value and returns `{ "msg_id" }`. `delay_secs` waits that many seconds before the message can be read. It defaults to 0. A negative delay is 400.

`read` hides each message until its visibility timeout and increments `read_ct`. The body is `{ "vt_secs", "qty" }`. The defaults are 30 seconds and 1 message. `qty` is capped at 100. Each row is `{ "msg_id", "message", "read_ct" }`.

`peek` returns the same shape and does not hide the message or increment `read_ct`.

`delete` removes one message by `msg_id` and returns 204. `archive` moves it out of the live queue and returns 204. Both are how a worker finishes a message it read itself.

## Subscriptions

A subscription names a function and how to read:

```json
{ "function_name": "echo", "vt_secs": 30, "qty": 1, "max_reads": 3 }
```

`POST` creates it and returns 201. `DELETE` with `{ "function_name" }` removes it and returns 204.

About once a second the server reads up to `qty` messages for each subscription and calls that function with the message JSON as stdin. A successful call deletes the message. A failed call leaves it. The visibility timeout brings it back. When `read_ct` reaches `max_reads`, the message is archived.

Each delivery writes a log row with kind `queue` and name `{queue}:{function}`. Status 200 has an empty message. Status 500 carries the invoke error. The function's own log is a separate `function` row.

## HTTP

Every route uses the service key. The project comes from the token.

| Method | Path | Body | Response |
| --- | --- | --- | --- |
| `POST` | `/queue/v1/queues` | `{ "name" }` | 201 `{ "name" }` |
| `GET` | `/queue/v1/queues` | | `[{ "name", "created_at" }]` |
| `POST` | `/queue/v1/queues/{name}/send` | `{ "message", "delay_secs" }` | 201 `{ "msg_id" }` |
| `POST` | `/queue/v1/queues/{name}/read` | `{ "vt_secs", "qty" }` | message rows |
| `GET` | `/queue/v1/queues/{name}/peek` | | message rows |
| `POST` | `/queue/v1/queues/{name}/delete` | `{ "msg_id" }` | 204 |
| `POST` | `/queue/v1/queues/{name}/archive` | `{ "msg_id" }` | 204 |
| `POST` | `/queue/v1/queues/{name}/subscriptions` | `{ "function_name", "vt_secs", "qty", "max_reads" }` | 201 |
| `DELETE` | `/queue/v1/queues/{name}/subscriptions` | `{ "function_name" }` | 204 |

A missing queue is 404. A duplicate name is 409.

## Clients

Pass the service key as the key you give `createClient`, `ReactorClient`, or the Kotlin constructor. The queue methods use that bearer token.

```ts
await reactor.queue.create("jobs")
const { msg_id } = await reactor.queue.send("jobs", { hello: "world" }, { delaySecs: 0 })
await reactor.queue.subscribe("jobs", { functionName: "echo", vtSecs: 30, qty: 1, maxReads: 3 })
const rows = await reactor.queue.read("jobs", { vtSecs: 30, qty: 1 })
await reactor.queue.delete("jobs", rows[0].msg_id)
```

Swift and Kotlin use the same names: `queue.create`, `queue.send`, `queue.read`, `queue.peek`, `queue.delete`, `queue.archive`, `queue.subscribe`, `queue.unsubscribe`, and `queue.list`. The option labels are `delaySecs`, `vtSecs`, and `maxReads`.

## CLI

From a linked project, these use `.reactor/service_key`:

```bash
reactor queue create jobs
reactor queue send jobs '{"hello":"world"}'
reactor queue send jobs '{"hello":"world"}' --delay-secs 10
reactor queue read jobs --vt-secs 30 --qty 1
reactor queue list
```

`send` parses the argument as JSON. If it is not JSON, the message is that string. Peek, delete, archive, and subscriptions are HTTP-only.

## Console

When the extension is on, Queue appears in the project sidebar for a developer or above. The sidebar lists queues and can create one. A selected queue has Messages and Subscribers. Messages can send a JSON body and shows id, payload, read count, and when the message is visible again. Subscribers lists the function, visibility timeout, quantity, and max reads.
