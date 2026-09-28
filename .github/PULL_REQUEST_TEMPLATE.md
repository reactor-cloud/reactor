## Summary

## Test plan

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace --exclude reactor-accept`
- [ ] `scripts/accept.sh` if the change touches a request path, SQL, or a client

Signed-off-by:
