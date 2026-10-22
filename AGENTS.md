# Notes for coding agents

[CONTRIBUTING.md](CONTRIBUTING.md) is the guide for everyone, agents included,
so read it first. This file adds what costs an agent more than a person: an
agent rebuilds far more often in one session, and runs in many checkouts at
once.

## Keep one copy of `target/`

Each build configuration keeps its own copy of every dependency in `target/`.
With the AWS SDK and Tauri in the tree, a copy runs to gigabytes, and one
agent session on a two-crate change has filled 25 GiB.

**Do not set `RUSTFLAGS` for a one-off run.** Changing it, or
`--config build.rustflags`, gives every crate a new hash, so cargo builds a
second full copy beside the first and keeps both.

The usual reason to reach for it is a revert check: breaking a fix on purpose
to see its test go red. That edit needs no flag. rustc does not warn on
`cond && false` or `cond || true`; only clippy does
(`overly_complex_bool_expr`), and the tests do not run clippy. If the broken
code must also get past clippy, hide the constant from it:

```rust
if !host_dir.exists() && std::hint::black_box(false) {
```

**Iterate on one package.** Use `-p` for the crate you changed while you work,
and run the whole workspace (`just test`, `just lint`) once before you push. A
bare cargo command also builds the QuiltSync desktop app, and `just lint` adds a
wasm32 build of `quilt-sync-ui`.
