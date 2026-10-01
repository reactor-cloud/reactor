---
title: CLI
description: Install reactor, then sign in, create projects, deploy, and migrate.
---

`reactor` is the command-line client for a cluster. It talks to `/console/v1` with a console session, and to the platform routes with an operator token. Use it to create the first operator, link a project directory, and ship SQL, functions, and a site.

Named sessions live in `~/.config/reactor/contexts.json`. `REACTOR_HOME` overrides that directory. `--context` or `REACTOR_CONTEXT` selects one for a single command.

Project commands read `reactor.toml` in the current directory and use the session whose URL matches that file.

## Install

Homebrew, on a Mac:

```sh
brew tap reactor-cloud/reactor
brew install reactor
```

`v1.26.10-beta5` builds from source and needs Rust, which Homebrew installs as a build dependency. The formula uses tag `v1.26.10-beta5` of [reactor-cloud/reactor](https://github.com/reactor-cloud/reactor).

From a clone of the server repository, at the repository root:

```sh
cargo install --locked --path crates/reactor-cli
```

That binary is named `reactor-cli`. Homebrew renames it to `reactor`. Either name runs the same commands.

## Session

```sh
reactor setup --url http://127.0.0.1:18000 --cluster local --email you@example.com --name "Ada Lovelace"
reactor login --context local --url http://127.0.0.1:18000 --email you@example.com
reactor login --context fly --url https://fly1.reactor.cloud --email you@example.com
reactor login --url http://127.0.0.1:18000 --token "$OPERATOR_TOKEN"
reactor context
reactor context use fly
reactor logout
```

`setup` is the first run. When it returns a session, that session is stored under the cluster name. `login` stores a named context and makes it current. Omit `--context` and the name comes from the host (`127.0.0.1` is `local`). A later login to the same URL updates that context. `login --email` stores the console token. `login --token` stores the platform operator token used by `db migrate` on the same context. A password flag or `REACTOR_PASSWORD` skips the prompt.

`context` lists contexts. `context use` changes the current one. `logout` clears the current context's tokens. `projects` and `cluster` use the current context. Project commands use the context that matches `reactor.toml`.

## Projects and keys

```sh
reactor projects
reactor projects create todos --link
reactor link --url http://127.0.0.1:18000 --ref <ref> --service-key <service key>
reactor keys rotate
```

`projects` lists projects this operator can open. `create` prints the ref, anon key, and service key once:

```text
ref: abcdefghijklmnopqrst
anon: eyJ...
service: eyJ...
```

`--link` writes `reactor.toml` and `.reactor/service_key`. `link` does the same write when you already have the key. `keys rotate` prints a new pair and rewrites `.reactor/service_key` when that file exists. The previous anon and service keys stop working.

## Service keys

```sh
reactor service-keys
reactor service-keys create factory --scope projects.create --scope auth.users
reactor service-keys revoke <id>
```

`service-keys` lists the signed-in operator's console service keys: id, name, scopes, and created time. `create` prints the token once. Repeat `--scope` for each scope: `projects.create`, `projects.migrate`, `auth.settings`, `auth.providers`, `auth.email`, `auth.users`. `revoke` deletes that key.

## Deploy and database

```sh
reactor deploy
reactor db migrate --all
reactor db tables
reactor db rows todos
```

`deploy` uses the service key in `.reactor/service_key`. It applies pending project migrations, uploads `functions/<name>/` as zips, and uploads `site/` as one deployment. A failed file upload does not promote the site.

`db migrate --all` needs the operator token from `login --token`. `--dry-run` lists project refs and does not apply SQL. `db tables` and `db rows` read the linked project through the console. `rows` prints up to 50 lines.

## Day to day

```sh
reactor logs
reactor users
reactor members
reactor members add --email dev@example.com --name "Dev" --role developer
reactor cluster
reactor functions
reactor functions promote ping 2
reactor functions demote ping
reactor sites
reactor sites env
reactor sites env set SITE_BANNER "hello" --visible
reactor sites env unset SITE_BANNER
reactor storage
```

`members add` creates an operator if the email is new. The role default is `developer`. `functions` lists deployments. Promote pins a version. Demote clears the pin. `sites env set` without `--visible` stores a secret, and the value is not returned later.

`cluster` prints liveness. Platform-admin actions on operators are in the console.
