default:
    @just --list

# Build Go binary
build:
    go build -o claude-switch ./cmd/

# Install Go binary to GOPATH/bin
install:
    go install ./cmd/

# Run Go tests
test:
    go test ./cmd/

# Run Go vet
vet:
    go vet ./cmd/

# Format Go code
fmt:
    gofmt -w cmd/

# Check Go formatting without modifying files
fmt-check:
    @test -z "$(gofmt -l cmd/)" || (echo "Files need formatting:"; gofmt -l cmd/; exit 1)

# Build Rust binary in release mode
build-rust:
    cargo build --release

# Install Rust binary globally
install-rust:
    cargo install --path .

# Uninstall Rust binary
uninstall-rust:
    cargo uninstall claude-switch

# Run Rust tests
test-rust:
    cargo test

# Run Rust clippy lints
lint-rust:
    cargo clippy -- -D warnings

# Format Rust code
fmt-rust:
    cargo fmt

# Check Rust formatting without modifying files
fmt-check-rust:
    cargo fmt -- --check
