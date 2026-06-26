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

.PHONY: build install run run-existing app-server mock-run mock-existing mock-app-server jaeger dev-deps test test-fast smoke registry-readmes publish-crates publish publish-verify release-brew update-homebrew-tap clean clean-target cargo-unlock

# Wipe the cargo target dir. Incremental builds accumulate session artifacts
# unboundedly (cargo does not GC them on stable); once target/ grows to hundreds
# of thousands of files the filesystem overhead dominates and builds crawl.
# A clean cold rebuild is ~20s, so run this whenever target/ gets bloated.
# Check size: du -sh target ; count files: find target -type f | wc -l
clean-target:
	@if pgrep -f "$(CURDIR)/target" >/dev/null 2>&1; then \
		echo "Refusing: cargo/rustc still running for this repo. Check: pgrep -fl '$(CURDIR)/target'" >&2; \
		exit 1; \
	fi
	cargo clean

# Drop stale target locks when no cargo/rustc is using this repo (see `make cargo-unlock-help`).
cargo-unlock:
	@if pgrep -f "$(CURDIR)/target" >/dev/null 2>&1; then \
		echo "Refusing: cargo/rustc still running for this repo. Check: pgrep -fl '$(CURDIR)/target'" >&2; \
		exit 1; \
	fi
	@rm -f target/debug/.cargo-lock target/release/.cargo-lock
	@echo "Removed target/debug and target/release .cargo-lock files."

cargo-unlock-help:
	@echo "If make build says 'Blocking waiting for file lock on artifact directory':"
	@echo "  1. pgrep -fl 'gode/target|cargo.*roder'   # find the holder (often rust-analyzer)"
	@echo "  2. Wait for it, or stop that job / reload the Cursor window"
	@echo "  3. make cargo-unlock   # only when step 1 shows nothing"
	@echo "rust-analyzer uses target/rust-analyzer (see .vscode/settings.json) to avoid fighting make build."

CARGO_TEST ?= $(shell command -v cargo-nextest >/dev/null 2>&1 && echo "cargo nextest run" || echo "cargo test")

# Unit tests only (skips integration `tests/*.rs` binaries such as app-server e2e).
TEST_FAST_ARGS ?= --workspace --lib
# Full workspace test; uses nextest when installed. Enables app-server e2e integration tests.
TEST_ARGS ?= --workspace --features roder-app-server/e2e-tests

build:
	@mkdir -p $(dir $(BINARY))
	cargo build -p roder --bin roder
	rm -f "$(BINARY)"
	cp target/debug/roder "$(BINARY)"
	@if [ "$$(uname)" = "Darwin" ]; then \
		codesign -f -s - "$(BINARY)" 2>/dev/null || true; \
	fi

install:
	cargo build --release -p roder --bin roder
	$(INSTALL) -d "$(BINDIR)"
	$(INSTALL) -m 0755 target/release/roder "$(BINDIR)/$(INSTALL_BIN)"
	@if [ "$(LEGACY_INSTALL_BIN)" != "$(INSTALL_BIN)" ] && [ -e "$(BINDIR)/$(LEGACY_INSTALL_BIN)" ]; then \
		rm -f "$(BINDIR)/$(LEGACY_INSTALL_BIN)"; \
	fi

run: build
	cd "$(WORKSPACE)" && RODER_PROVIDER="$(PROVIDER)" RODER_MODEL="$(MODEL)" RODER_REASONING="$(REASONING)" "$(CURDIR)/$(BINARY)"

run-existing:
	@test -x "$(BINARY)" || { echo "$(BINARY) does not exist; run 'make build' once first" >&2; exit 1; }
	cd "$(WORKSPACE)" && RODER_PROVIDER="$(PROVIDER)" RODER_MODEL="$(MODEL)" RODER_REASONING="$(REASONING)" "$(CURDIR)/$(BINARY)"

app-server:
	cd "$(WORKSPACE)" && RODER_PROVIDER="$(PROVIDER)" RODER_MODEL="$(MODEL)" RODER_REASONING="$(REASONING)" cargo run --manifest-path "$(CURDIR)/Cargo.toml" -p roder --bin roder -- app-server --listen "$(LISTEN)"

mock-run: build
	cd "$(WORKSPACE)" && RODER_PROVIDER="mock" RODER_MODEL="$(MODEL)" RODER_REASONING="$(REASONING)" "$(CURDIR)/$(BINARY)"

mock-existing:
	@test -x "$(BINARY)" || { echo "$(BINARY) does not exist; run 'make build' once first" >&2; exit 1; }
	cd "$(WORKSPACE)" && RODER_PROVIDER="mock" RODER_MODEL="$(MODEL)" RODER_REASONING="$(REASONING)" "$(CURDIR)/$(BINARY)"

mock-app-server:
	cd "$(WORKSPACE)" && RODER_PROVIDER="mock" RODER_MODEL="$(MODEL)" RODER_REASONING="$(REASONING)" cargo run --manifest-path "$(CURDIR)/Cargo.toml" -p roder --bin roder -- app-server --listen "$(LISTEN)"

jaeger:
	./jaeger.sh

dev-deps:
	cargo install cargo-nextest --locked

test-fast:
	$(CARGO_TEST) $(TEST_FAST_ARGS)

test:
	$(CARGO_TEST) $(TEST_ARGS)

smoke: test

registry-readmes:
	python3 scripts/check-registry-readmes.py

publish-crates:
	python3 scripts/publish-crates.py

publish:
	./scripts/publish-latest-roder.sh

publish-verify:
	./scripts/verify-latest-roder.sh

release-brew:
	./scripts/release-brew.sh $(VERSION)

update-homebrew-tap:
	./scripts/update-homebrew-tap.sh

clean:
	rm -rf bin .gode
