# Notes for coding agents

[CONTRIBUTING.md](CONTRIBUTING.md) is the guide for everyone; read it first.
This file adds what costs an agent more than a person: an agent rebuilds far
more often in one session, and runs in many checkouts at once.

## Keep one copy of `target/`

Cargo keeps a separate copy of a dependency in `target/` for each way it is
built. With the AWS SDK and Tauri in the tree, one agent session on a
two-crate change has filled 25 GiB.

**Do not set `RUSTFLAGS` for a one-off run.** Changing it, or
`--config build.rustflags`, gives every crate a new hash, so cargo builds a
second full copy beside the first and keeps both.

The usual reason to reach for it is a revert check: breaking a fix on purpose
to see its test go red. That edit needs no flag. rustc does not warn on
`cond && false` or `cond || true`; only clippy does
(`overly_complex_bool_expr`), and the tests do not run clippy. If the broken
code must also get past a lint, silence it in the source, never with a flag.
For this clippy lint, hide the constant from it:

```rust
if !host_dir.exists() && std::hint::black_box(false) {
```

**Build what `just` builds.** `just test` passes `--workspace --all-targets`.
`-p` or a bare cargo command selects fewer packages, so cargo resolves shared
dependencies such as `tokio` and `syn` with other features, and builds them and
everything that depends on them again.
