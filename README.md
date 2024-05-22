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

## JNI

```sh
cargo build && javac Quilt.java && java -Djava.library.path=./target/debug Quilt
mkdir -p target/classes/com/quiltdata/quiltcore
cp Quilt.class target/classes/com/quiltdata/quiltcore/

```

### Create and Install JAR file

Requires `maven` and `JDK` to be installed.
e.g., `brew install maven openjdk`

```sh
mvn package
mvn install:install-file -Dfile=target/quiltcore-0.2.1.jar -DpomFile=pom.xml
jar tf target/quiltcore-0.2.1.jar
```

This will install the jar in the [local Maven repository](https://maven.apache.org/guides/mini/guide-3rd-party-jars-local.html).


