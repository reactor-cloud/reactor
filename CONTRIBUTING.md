# Contributing

Reactor is source-available under the Business Source License 1.1. Contributions are licensed under that same license. You keep copyright in your patch.

Add a `Signed-off-by` line to each commit, using your real name:

```
Signed-off-by: Your Name <you@example.com>
```

That line means you can submit the change under the project license (Developer Certificate of Origin). There is no separate contributor license agreement.

## Setup

From the repository root (this directory in the public repo, `v2/` in the lab repo):

```sh
docker compose -f deploy/compose/compose.yaml up -d
```

## Bar for a pull request

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --exclude reactor-accept
scripts/accept.sh
```

`scripts/accept.sh` is the merge bar. It needs Docker and the AWS SAM CLI. If `sam` is missing, the script tells you how to install it and stops. It does not install packages for you.

Unit tests skip the acceptance suite unless `REACTOR_ACCEPT=1`. That variable is set by `scripts/accept.sh`.
