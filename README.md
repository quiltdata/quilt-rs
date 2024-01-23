# quilt-rs

Rust library for accessing Quilt data packages.

## Testing

```bash
cargo test
cargo install cargo-watch
cargo watch # -x test
```

## Publishing

```bash
cargo update
cargo test
cargo publish
```

## Coverage

```bash
cargo install taurpalin
cargo taurpalin
# OR
cargo run --release -- tarpaulin --engine llvm --follow-exec --post-test-delay 10
```
