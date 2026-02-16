default:
    @just --list

# Install claude-switch globally
install:
    cargo install --path .

# Uninstall claude-switch
uninstall:
    cargo uninstall claude-switch

# Build in release mode
build:
    cargo build --release

# Run tests
test:
    cargo test

# Run clippy lints
lint:
    cargo clippy -- -D warnings

# Format code
fmt:
    cargo fmt

# Check formatting without modifying files
fmt-check:
    cargo fmt -- --check
