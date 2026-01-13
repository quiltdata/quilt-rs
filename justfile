# Simple justfile for quilt-rs workspace

# Start QuiltSync development server
start:
    cd quilt-sync && npm start

# Run test coverage for all packages
coverage:
    cargo tarpaulin --out html

# Lint all packages with all features
lint:
    cargo clippy --all-features -- --deny warnings
