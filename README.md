# Reactor

## Rust backend for web and mobile apps. 
Host your own robust integrated backend with auth, data (PostgREST), file storage, functions, and sites. Deploy on AWS or Fly, or run it locally with Docker. 
Replace Supabase + Vercel with a self hostable binary and an integrated CLI to improve AI deployment workflows.

[reactor.cloud](https://www.reactor.cloud)

**v1.26.09-beta.2** (package version `1.26.9-beta.2`). The API can still change before a stable tag.

Postgres and a blob store are the only dependencies. One stateless server serves every surface.

## Start

From this directory:

```sh
docker compose -f deploy/compose/compose.yaml build keys
docker compose -f deploy/compose/compose.yaml up -d
```

The app listens on `http://127.0.0.1:18000`. The console is `http://127.0.0.1:18000/console`. Health is `GET /health`.

Do not start the `keys` service on its own. That command only writes the JWT key.

## What you get


| Surface   | Path          | What it is                                       |
| --------- | ------------- | ------------------------------------------------ |
| Auth      | `/auth/v1`    | Users, sessions, magic links, recovery, invites  |
| Data      | `/data/v1`    | PostgREST, with row-level security               |
| Storage   | `/storage/v1` | Presigned uploads and downloads                  |
| Functions | `/fn/v1`      | Bun or Lambda, with versions and pins            |
| Sites     | project host  | Static files, a site process, or a function path |
| Console   | `/console`    | Cluster admin. Separate from project users       |


Docs: [concepts](docs/start/concepts.md), [quickstart](docs/start/quickstart.md), [self-hosting](docs/operate/self-hosting.md), [AWS](docs/operate/lambda.md).

Clients:

```sh
npm install @reactor-cloud/client@beta
```

Swift package `https://github.com/reactor-cloud/reactor-swift` at `1.26.9-beta.2`. Maven `sl.atomicollabs.reactor:reactor-client:1.26.9-beta.2`. The CLI is `brew tap reactor-cloud/reactor && brew install reactor`. Source for all of them is this tag, `v1.26.09-beta.2`.

## Not included

Realtime, analytics, and billing are not part of this release. There is no SQL editor and no second data API. Console MFA is for operators. Project users can verify email, enroll a second factor, and sign in with a configured OAuth provider.

## Tests

`scripts/accept.sh` is the merge bar. It needs Docker, the AWS SAM CLI, and a local Rust toolchain. Unit tests alone do not prove two-project isolation.

## License

Reactor uses a permissive license. You can use it as the backend for as many personal or commercial projects as you want, and you can set it up for a client who runs it themselves. The only restriction is offering competing hosting: you may not provide Reactor’s auth, data, storage, functions, or sites to other people as a hosted service without a commercial license from AtomicoLabs SL.

Business Source License 1.1. Copyright 2026 AtomicoLabs SL and Claudio del Conde. This version becomes Apache 2.0 on 2030-09-28. See [LICENSE](LICENSE).