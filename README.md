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
cargo tarpaulin --out html
open tarpaulin-report.html
```

## Update Dependencies

```bash
cargo install cargo-upgrades
cargo upgrades
```

## Verify files integrity

* `sha256sum` calculates SHA256 hash of a file.
* `base64` converts binary data to base64.
* `xxd -r -p` converts HEX produced by SHA256 to binary
* `split -b 8388608` splits file into `8 * 1024 * 1024` bytes

### 0Mb

```bash
sha256sum ./FILE | xxd -r -p | base64
```

### <= 8Mb

```bash
sha256sum ./FILE | xxd -r -p | sha256sum | xxd -r -p | base64
```

### > 8Mb

```bash
split -b 8388608 ./FILE --filter='sha256sum' | xxd -r -p | sha256sum | xxd -r -p | base64
```


## Verify packages

```bash
split -l 1 ~/MANIFEST.jsonl --filter="jq -cSM 'del(.physical_keys)'" | tr -d '\n' | sha256sum
```
