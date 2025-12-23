# Rust Coverage

Pay attention to the `Running: ****target/debug/deps/**** line` in the end

```sh
RUSTFLAGS="-C instrument-coverage" cargo test
```

Merge *profraw files

```sh
llvm-profdata merge -sparse default_*.profraw -o quiltsync.profdata
```

Show reports using target object from the output of the first command

```sh
llvm-cov report \
    --use-color \
    --ignore-filename-regex='/.cargo/registry' \
    --ignore-filename-regex='/*src-tauri/target*' \
    --ignore-filename-regex='/rustc/*' \
    --instr-profile=quiltsync.profdata \
    --object target/debug/deps/quilt_sync-****
```

Show locations in the code:

```sh
llvm-cov show \
    --use-color \
    --ignore-filename-regex='/.cargo/registry' \
    --ignore-filename-regex='/*src-tauri/target*' \
    --ignore-filename-regex='/rustc/*' \
    --instr-profile=quiltsync.profdata \
    --object target/debug/deps/quilt_sync-**** \
    --show-instantiations --show-line-counts-or-regions \
    --Xdemangler=rustfilt | less -R
```
