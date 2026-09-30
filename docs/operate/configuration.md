---
title: Configuration
description: Reactor.toml and the REACTOR_ environment variables that override it.
---

Configuration is how a cluster finds Postgres, the blob store, and the signing key. The server reads `Reactor.toml` from the working directory, then environment variables. The prefix is `REACTOR_`. A double underscore is a nested key: `REACTOR_DATABASE__URL` overrides `[database] url`.

An empty auth provider is a startup error. The provider is `internal`.

```toml
[runtime]
mode = "listen"          # listen | lambda
place = "docker"         # shown in the console: docker, aws, fly
base_domain = "apps.localhost"

[http]
bind = "0.0.0.0:8000"
public_url = "http://127.0.0.1:18000"

[auth]
provider = "internal"
operator_token = "change-me"
jwt_private_pem_file = "/keys/private.pem"

[database]
url = "postgres://reactor:reactor@postgres:5432/reactor"
dedicated_url = "postgres://reactor:reactor@postgres-dedicated:5432/reactor"
authenticator_password = "authenticator"

[storage]
backend = "s3"           # fs | s3
bucket = "reactor"
endpoint = "http://minio:9000"
public_endpoint = "http://127.0.0.1:19000"
access_key = "minio"
secret_key = "minioadmin"
region = "us-east-1"
sign_secret = "change-me"

[functions]
runtime = "bun"          # bun | lambda
workdir = "/data/functions"
bun = "/usr/local/bin/bun"
publisher = "fake"

[postgrest]
url = "http://127.0.0.1:3000"
dedicated = "http://127.0.0.1:3001"
```

`[runtime]` chooses a long-lived process or Lambda, and the label the console shows for where the cluster runs. `[http]` is the listen address and the public URL clients are told. `[auth]` is the operator token and the Ed25519 key. `[database]` is the shared Postgres and, when you use it, the second Postgres for a dedicated project database. `[storage]` is either the local filesystem or S3. `[functions]` chooses Bun in this process or a publisher that turns each function into its own Lambda. `[postgrest]` is where `/data/v1` proxies.

## Names that matter

| Key | Default | Effect |
| --- | --- | --- |
| `runtime.mode` | `listen` | `lambda` selects the handler process instead of binding a port |
| `runtime.base_domain` | `apps.localhost` | `{ref}.{base_domain}` is a project host |
| `http.bind` | `0.0.0.0:8000` | Listen address. Compose publishes it as `18000` |
| `http.public_url` | `http://127.0.0.1:18000` | Absolute URL the server tells clients |
| `http.trusted_proxy` | false | When true, auth rate limits use the first `X-Forwarded-For` address. Set `REACTOR_HTTP__TRUSTED_PROXY=1` only behind an edge you control |
| `auth.operator_token` | empty | Required for `POST /platform/v1/projects` and migrate |
| `auth.jwt_private_pem_file` | — | Ed25519 private key. Otherwise keys are created under `jwt_dir` (`.data/keys`) |
| `database.url` | empty | Shared Postgres. Required |
| `storage.backend` | `fs` | `fs` uses `fs_root`. `s3` uses the bucket settings |
| `storage.sign_secret` | the operator token, or `dev-sign` | Signs filesystem presigned URLs |
| `functions.runtime` | `bun` | `lambda` publishes user functions instead of spawning Bun |
| `postgrest.url` | — | Where `/data/v1` proxies |

`REACTOR_SQL_DIR` points at the `sql/` tree. `REACTOR_HANDLER` is set only in Lambda. `REACTOR_DNS_STUB_FILE` is a local stand-in for DNS checks.

Auth mail is not in this file. SMTP credentials live in Postgres and are written from the console. The cluster has one server (host, port, username, password, From, TLS). A project has its own server plus a link base. Mail uses the project host when it is set, otherwise the cluster server when that project is allowed to use it. The password is not returned after it is saved.

Site idle time is `REACTOR_SITES__IDLE_SECS`. The agent in the console reads `REACTOR_AGENT__API_KEY`, `REACTOR_AGENT__BASE_URL`, and `REACTOR_AGENT__MODEL`. Leave the key empty and the agent does not call a model.

Do not commit `Reactor.toml` when it contains the operator token or storage keys. Compose in this repo passes those as environment variables instead.
