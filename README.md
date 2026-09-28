# Reactor

Source-available backend for web and mobile apps. One stateless server serves auth, Postgres data through PostgREST, file storage, functions, and sites. Postgres and a blob store are the only dependencies.

**v1.26.09-beta.1** (package version `1.26.9-beta.1`). The API can still change before a stable tag. On 2030-09-28 this version becomes Apache 2.0.

You may run Reactor for your own product, including a commercial one, and you may set it up for a client who operates it. You may not offer Reactor’s auth, data API, storage, functions, or sites to third parties as a hosted service without a commercial license from AtomicoLabs SL. See [LICENSE](LICENSE).

## Start

From this directory:

```sh
docker compose -f deploy/compose/compose.yaml build keys
docker compose -f deploy/compose/compose.yaml up -d
```

The app listens on `http://127.0.0.1:18000`. The console is `http://127.0.0.1:18000/console`. Health is `GET /health`.

Do not start the `keys` service on its own. That command only writes the JWT key.

## What you get

| Surface | Path | What it is |
| --- | --- | --- |
| Auth | `/auth/v1` | Users, sessions, magic links, recovery, invites |
| Data | `/data/v1` | PostgREST, with row-level security |
| Storage | `/storage/v1` | Presigned uploads and downloads |
| Functions | `/fn/v1` | Bun or Lambda, with versions and pins |
| Sites | project host | Static files, a site process, or a function path |
| Console | `/console` | Cluster admin. Separate from project users |

Docs: [concepts](docs/start/concepts.md), [quickstart](docs/start/quickstart.md), [self-hosting](docs/operate/self-hosting.md).

Clients:

```sh
npm install @reactor/client@beta
```

Swift package `https://github.com/reactor-cloud/reactor-swift` at `1.26.9-beta.1`. Maven `sl.atomicollabs.reactor:reactor-client:1.26.9-beta.1`. The CLI is `brew tap reactor-cloud/reactor && brew install reactor`. Source for all of them is this tag, `v1.26.09-beta.1`.

## Not included

Realtime, OAuth, project-user MFA, analytics, and billing are not part of this release. Console MFA is for operators.

## Tests

`scripts/accept.sh` is the merge bar. It needs Docker, the AWS SAM CLI, and a local Rust toolchain. Unit tests alone do not prove two-project isolation.

## License

Business Source License 1.1. Copyright 2026 AtomicoLabs SL and Claudio del Conde.
