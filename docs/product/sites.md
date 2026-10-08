---
title: Sites
description: Host a project's files, a small process, or a path that calls a function.
---

A site is the hostname where a project’s files, and optionally a small process or a function, are served. Platform hosts are `{ref}.{base_domain}`. A custom domain is a verified row. `reactor deploy` uploads the `site/` directory in the linked project and makes it live only after every file succeeds.

## What a request hits

For a site host, Reactor looks up the path in this order:

1. A static file uploaded with the current deployment. `/` is `index.html`.
2. If the deployment has a command, the request is proxied to that process.
3. Otherwise a site route can invoke a function and return its stdout as JSON.
4. Anything else is 404.

`..` in the path is rejected.

## Deploy static files

`reactor deploy` looks for a `site/` directory.

1. `POST /sites/v1/deployments` with the service key starts a deployment.
2. `POST /sites/v1/deployments/{id}/files/{path}` returns `{ "url", "fallback" }`. `url` is a presigned PUT for that object. `fallback` is the listen-mode upload URL.
3. `PUT` the file bytes to `url`. If that fails and the file is at most 10 MB, `PUT` the bytes to `fallback` instead. A larger file has no body fallback.
4. `POST /sites/v1/deployments/{id}/files/{path}/confirm` records the file after the object exists. Confirming the same path again is the same row.
5. `POST /sites/v1/deployments/{id}/finish` marks it ready and makes it the live set.
6. A failed upload calls `POST /sites/v1/deployments/{id}/fail`. A deployment goes live only when every file succeeds. Until finish, the previous live set keeps serving. A path with `..` is 403 `{ "error": "invalid path" }`.

`PUT /sites/v1/deployments/{id}/files/{path}` still accepts the bytes itself. That is `fallback`. Listen mode accepts 10 MB on that route. Lambda stays at 2 MB, so a client talking to the API Lambda uses the presigned URL, and `fallback` is `http.public_url` (the ALB on AWS).

Files are objects in the blob store. The server sets a content type from the extension: html, css, js, json, svg, png, txt, or bytes.

You can also `PUT /sites/v1/files/{path}` with the service key for a single file outside a deployment.

## A site process

Finish a deployment with a command and Reactor starts that process for unmatched paths. The command is a single program and arguments, not a shell. `node server.js` and `bun src/index.ts` are accepted. Characters outside letters, numbers, space, `.`, `_`, `/`, and `-` are rejected, and the command is capped at 200 characters.

The process is idle-stopped (`REACTOR_SITES__IDLE_SECS`). It receives the site's variables. Static files still win over the process.

## A function path

When there is no file and no command:

```http
POST /sites/v1/routes
Authorization: Bearer <service key>
Content-Type: application/json

{ "path": "api/ping", "function": "ping" }
```

A request to that path invokes `ping`. The caller JSON is `{ "sub": "site", "ref": "<ref>", "role": "anon" }`.

## Domains

`POST /sites/v1/domains` with `{ "host": "app.example.com" }` and the service key returns the TXT record:

```json
{
  "host": "app.example.com",
  "token": "...",
  "txt_name": "_reactor-verify.app.example.com",
  "txt_value": "reactor-site-verification=..."
}
```

`POST /sites/v1/domains/{host}/verify` checks DNS. After verification, `GET /ask?domain=app.example.com` returns 200 so the proxy can issue a certificate. Deleting the domain makes `/ask` return 404. Reactor does not terminate the custom-domain TLS.

## Variables

Site variables are encrypted. A variable is secret unless it is marked visible. A secret is never read back. `GET /sites/v1/env` and `PUT /sites/v1/env` are the service-key API. The CLI is `reactor sites env set <key> <value>` and `--visible` when the value may be shown again.

## Logs

Page loads append a `reactor.logs` row with kind `site`. The console chart on the site overview is the last 24 hours of those lines.
