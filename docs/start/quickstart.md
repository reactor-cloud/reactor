---
title: Quickstart
description: Run a local cluster, create a project, and deploy the todos example.
---

This page gets a cluster running on your machine and a small app deployed to it. You need Docker. Install the CLI with Homebrew, or from a clone if you are not on a Mac. Commands below assume the repository root, the directory that contains `deploy/` and `examples/`.

```sh
brew tap reactor-cloud/reactor
brew install reactor
```

## Start the cluster

```sh
docker compose -f deploy/compose/compose.yaml build keys
docker compose -f deploy/compose/compose.yaml up -d
```

`build keys` builds the app image and generates the signing key. Do not leave the `keys` service running; Compose runs it once and it exits. The app listens on `http://127.0.0.1:18000`. A second replica listens on `18001`.

`GET /health` answers when Postgres and the blob store are reachable. The body is small. A 200 means both dependencies answered.

## Create the console

Open `http://127.0.0.1:18000/console`. The first screen asks for a cluster name and the first operator. That operator is a platform admin. There is no public signup after that.

From the CLI, the same first step is:

```sh
reactor setup --url http://127.0.0.1:18000 --cluster local --email you@example.com --name "Ada Lovelace"
```

The password is prompted, or pass `--password`, or set `REACTOR_PASSWORD`. The session is stored as the `local` context in `~/.config/reactor/contexts.json`. `REACTOR_HOME` overrides that directory.

## Create a project

```sh
reactor projects create todos --link
```

The command prints the project ref, the anon key, and the service key once. `--link` writes `reactor.toml` and `.reactor/service_key` in the current directory. Keep `.reactor/` out of git. The anon key is what clients embed. The service key deploys.

`reactor.toml` looks like this:

```toml
url = "http://127.0.0.1:18000"
ref = "yourprojectref"
```

## Deploy an app

The [todos example](/examples/todos/) is `examples/todos`. From that directory, after linking:

```sh
reactor deploy
```

Deploy applies `sql/project/` to this project, uploads each directory under `functions/` as a zip, and uploads `site/` as a deployment that goes live only when every file succeeds.

The site is then at:

```text
http://{ref}.apps.localhost:18000/
```

Signup on that page calls `/auth/v1` on the same host with the anon key. Todos are rows in `todos`. The page also calls `POST /fn/v1/ping`.

## Next

- [Projects](/start/projects/) for keys, schemas, and a dedicated database.
- [CLI](/operate/cli/) for the rest of the commands and the other ways to install it.
- [Configuration](/operate/configuration/) when you leave the Compose defaults.
