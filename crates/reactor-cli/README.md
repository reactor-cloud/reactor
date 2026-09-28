# reactor

Command line for a Reactor cluster. Sign in, create a project, and deploy functions, sites, and migrations.

```sh
brew tap reactor-cloud/reactor
brew install reactor
```

[reactor.cloud](https://www.reactor.cloud)

From a clone of the server repo you can also run `cargo install --path crates/reactor-cli` and use the `reactor-cli` binary.

## First session

```sh
reactor setup --url http://127.0.0.1:18000 --cluster local --email you@example.com --name "Ada Lovelace"
reactor projects create todos --link
reactor deploy
```

`setup` stores a console session named `local`. `projects create --link` writes `reactor.toml` and `.reactor/service_key` in the current directory. `deploy` applies pending migrations, uploads each `functions/<name>/` directory, and uploads `site/`.

A password flag or `REACTOR_PASSWORD` skips the prompt.

## Commands

| Command | What it does |
| --- | --- |
| `login --email` | Store a console session. The name comes from the host unless you pass `--context` |
| `login --token` | Store the operator token used by `db migrate` |
| `context` / `context use` | List sessions, or switch the current one |
| `logout` | Clear the current session |
| `projects` | List projects you can open |
| `link` | Write `reactor.toml` for a project you already have |
| `keys rotate` | Print a new anon and service key |
| `db migrate --all` | Apply control-plane SQL. `--dry-run` only lists refs |
| `db tables` / `db rows <table>` | Read the linked project |
| `functions` / `functions promote` / `functions demote` | List deployments, or pin and unpin a version |
| `sites` / `sites env set` / `sites env unset` | Site status and variables. Omit `--visible` to store a secret |
| `logs` / `users` / `storage` / `cluster` | Recent lines, project users, buckets, cluster health |
| `members add` | Invite an operator. Role defaults to `developer` |

`--context` picks a session for one command and does not change the current one. Project commands use the session whose URL matches `reactor.toml`.

Sessions live in `~/.config/reactor/contexts.json`. `REACTOR_HOME` moves that directory.

More detail: [CLI](../../docs/operate/cli.md).

## License

You can use Reactor as the backend for as many personal or commercial projects as you want. The license only restricts offering it as a competing hosted service.

Business Source License 1.1. Copyright 2026 AtomicoLabs SL and Claudio del Conde. See [LICENSE](../../LICENSE).
