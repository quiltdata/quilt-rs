# Notes for coding agents

[CONTRIBUTING.md](CONTRIBUTING.md) is the guide for everyone; read it first.
This file adds what costs an agent more than a person, who rebuilds far less
often in one session.

## Don't multiply `target/`

Cargo keeps a separate copy of a dependency in `target/` for each way it is
built. With the AWS SDK and Tauri in the tree, one agent session on a
two-crate change has filled 25 GiB.

**Do not change rustflags for a one-off run.** Setting `RUSTFLAGS`, or any
other source of them such as `CARGO_TARGET_<triple>_RUSTFLAGS` or
`--config build.rustflags`, gives every crate a new hash, so cargo builds a
second full copy beside the first and keeps both.

The usual reason to reach for them is a revert check: breaking a fix on purpose
to see its test go red. That edit needs no flag. rustc does not warn on
`cond && false` or `cond || true`; only clippy does
(`overly_complex_bool_expr`), and the tests do not run clippy. If the broken
code trips a lint, and rustc's warnings are errors here, silence it in the
source (`#[allow(...)]` on the item), never with a flag. For this clippy lint,
hide the constant from it:

```rust
if cond && std::hint::black_box(false) {
```

**Iterate with the selection `just test` uses**, `--workspace --all-targets`.
Cargo resolves a shared dependency's features from the packages, targets and
features it builds, so any other selection, such as `-p` or a bare cargo
command, can build another copy of it and of everything that depends on it.
To run one test, filter the run, not the build:
`cargo nextest run --workspace --all-targets -E 'test(name)'`.
