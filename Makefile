.PHONY: build web test fmt clippy check run

# Build the web UI into crates/server/ui (embedded by the server) and then
# the whole workspace. Run `make web` alone to rebuild just the dashboard.
build: web
	cargo build --workspace

web:
	cd web && npm install && npm run build

test:
	cargo test --workspace

fmt:
	cargo fmt --all

clippy:
	cargo clippy --all-targets --all-features -- -D warnings

check:
	cargo fmt --all --check
	cargo clippy --all-targets --all-features -- -D warnings
	cargo test --workspace

run:
	cargo run -p velos-server
