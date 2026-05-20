BINARY ?= bin/roder
BINDIR ?= $(HOME)/.local/bin
INSTALL ?= install
INSTALL_BIN ?= roder
LEGACY_INSTALL_BIN ?= rode
WORKSPACE ?= .
PROVIDER ?=
MODEL ?=
REASONING ?=
LISTEN ?= stdio://
VERSION ?=

.PHONY: build install run app-server mock-run mock-app-server jaeger dev-deps test test-fast smoke release-brew clean hakari-update

CARGO_TEST ?= $(shell command -v cargo-nextest >/dev/null 2>&1 && echo "cargo nextest run" || echo "cargo test")

# Unit tests only (skips integration `tests/*.rs` binaries such as app-server e2e).
TEST_FAST_ARGS ?= --workspace --lib
# Full workspace test; uses nextest when installed. Enables app-server e2e integration tests.
TEST_ARGS ?= --workspace --features roder-app-server/e2e-tests

build:
	@mkdir -p $(dir $(BINARY))
	cargo build -p roder-cli --bin roder
	cp target/debug/roder "$(BINARY)"

install:
	cargo build --release -p roder-cli --bin roder
	$(INSTALL) -d "$(BINDIR)"
	$(INSTALL) -m 0755 target/release/roder "$(BINDIR)/$(INSTALL_BIN)"
	@if [ "$(LEGACY_INSTALL_BIN)" != "$(INSTALL_BIN)" ] && [ -e "$(BINDIR)/$(LEGACY_INSTALL_BIN)" ]; then \
		rm -f "$(BINDIR)/$(LEGACY_INSTALL_BIN)"; \
	fi

run: build
	cd "$(WORKSPACE)" && RODER_PROVIDER="$(PROVIDER)" RODER_MODEL="$(MODEL)" RODER_REASONING="$(REASONING)" "$(CURDIR)/$(BINARY)"

app-server:
	cd "$(WORKSPACE)" && RODER_PROVIDER="$(PROVIDER)" RODER_MODEL="$(MODEL)" RODER_REASONING="$(REASONING)" cargo run --manifest-path "$(CURDIR)/Cargo.toml" -p roder-cli --bin roder -- app-server --listen "$(LISTEN)"

mock-run: build
	cd "$(WORKSPACE)" && RODER_PROVIDER="mock" RODER_MODEL="$(MODEL)" RODER_REASONING="$(REASONING)" "$(CURDIR)/$(BINARY)"

mock-app-server:
	cd "$(WORKSPACE)" && RODER_PROVIDER="mock" RODER_MODEL="$(MODEL)" RODER_REASONING="$(REASONING)" cargo run --manifest-path "$(CURDIR)/Cargo.toml" -p roder-cli --bin roder -- app-server --listen "$(LISTEN)"

jaeger:
	./jaeger.sh

dev-deps:
	cargo install cargo-nextest --locked
	cargo install cargo-hakari --locked

hakari-update:
	cargo hakari generate
	cargo hakari manage-deps -y

test-fast:
	$(CARGO_TEST) $(TEST_FAST_ARGS)

test:
	$(CARGO_TEST) $(TEST_ARGS)

smoke: test

release-brew:
	./scripts/release-brew.sh $(VERSION)

clean:
	rm -rf bin .gode
