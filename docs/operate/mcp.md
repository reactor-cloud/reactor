---
title: MCP
description: Connect Claude, Cursor, or Codex to a cluster. The refresh token lasts 30 days.
---

Each cluster hosts its own MCP endpoint at `https://<cluster>/mcp`. There is no shared broker. A client signs in once in the browser, keeps a refresh token for 30 days, and calls the tools that operator is allowed to use.

The access token lasts one hour. Its audience is `{public_url}/mcp`. A console JWT, a console service key, and a project user JWT are rejected on `/mcp`.

## Install

Add the cluster URL as a remote MCP server. The client discovers `/.well-known/oauth-authorization-server` and `/.well-known/oauth-protected-resource` on that host.

Claude Code:

```sh
claude mcp add --transport http reactor https://<cluster>/mcp
```

Cursor and Codex take the same URL. One entry per cluster. A token from one cluster is not accepted by another.

```json
{
  "mcpServers": {
    "reactor": { "url": "https://<cluster>/mcp" }
  }
}
```

The client registers itself as a public client. There is no client secret. It opens the authorize page, stores the refresh token, and sends `offline_access` when it asks for a scope. An empty scope becomes `operator offline_access`.

## Sign-in

The authorize page is the console login. Owners, admins, and platform admins finish a second factor before a code is issued: a passkey, an authenticator code, or a backup code. A developer with no such membership goes to the consent screen after the password.

The authorization code lasts 60 seconds and works once. The client exchanges it for an access token and a refresh token.

## Sessions

The refresh token is opaque. The cluster stores only its hash. It lasts 30 days. Every use rotates it and returns a new one. Sending a refresh token that was already rotated deletes that whole family, so the next call must sign in again.

Revoke a browser session from the console, under Agents. That deletes the refresh family. The access token still lasts until its hour ends, and the next refresh fails.

A client that cannot open a browser can use a key minted on that same page. The key is shown once and sent as `Authorization: Bearer rtr_mcp_...`. It lasts until you revoke it. It is not a console service key.

## Scopes

`operator` can do what that operator can do in the console, including creating projects, running SQL, deploying functions, and publishing site files. `operator:read` can list and read. A write tool returns an error without changing anything. If both scopes are present, `operator` wins. Two operators on one cluster cannot see each other's projects.

## Tools

The tools call the console checks in the cluster process. They do not shell out to the CLI and they do not forward the client bearer.

Reads include the signed-in operator, projects, members, schema, rows, logs, functions, sites, buckets, queues, and cluster health for a platform admin. Writes include creating a project, SQL, migrations, functions, site files, storage, queues, and members.

`create_project` returns `ref` and `anon`. The service key stays on the cluster. Rotating project keys returns no key material. Logs and tool results do not include passwords, service keys, or secret env values.

The cluster also serves `/favicon.png` so the client can show the Reactor mark before a session exists.
