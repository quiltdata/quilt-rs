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
code trips a lint (rustc's warnings are errors here), silence it in the source
with `#[allow(...)]` on the item, never with a flag. For this clippy lint,
hide the constant from it:

```rust
if cond && std::hint::black_box(false) {
```

**Iterate with the selection `just test` uses**, `--workspace --all-targets`;
CONTRIBUTING.md says why `-p` costs another copy. To run one test, filter the
run, not the build: `just test no-aws -E 'test(name)'`, or for a `live_*`
test, which the `no-aws` profile filters out,
`cargo nextest run --workspace --all-targets -E 'test(name)'`.
