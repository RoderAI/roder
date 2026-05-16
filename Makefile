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

.PHONY: build install run app-server mock-run mock-app-server jaeger test smoke release-brew clean

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

run:
	cd "$(WORKSPACE)" && RODER_PROVIDER="$(PROVIDER)" RODER_MODEL="$(MODEL)" RODER_REASONING="$(REASONING)" cargo run --manifest-path "$(CURDIR)/Cargo.toml" -p roder-cli --bin roder

app-server:
	cd "$(WORKSPACE)" && RODER_PROVIDER="$(PROVIDER)" RODER_MODEL="$(MODEL)" RODER_REASONING="$(REASONING)" cargo run --manifest-path "$(CURDIR)/Cargo.toml" -p roder-cli --bin roder -- app-server --listen "$(LISTEN)"

mock-run:
	cd "$(WORKSPACE)" && RODER_PROVIDER="mock" RODER_MODEL="$(MODEL)" RODER_REASONING="$(REASONING)" cargo run --manifest-path "$(CURDIR)/Cargo.toml" -p roder-cli --bin roder

mock-app-server:
	cd "$(WORKSPACE)" && RODER_PROVIDER="mock" RODER_MODEL="$(MODEL)" RODER_REASONING="$(REASONING)" cargo run --manifest-path "$(CURDIR)/Cargo.toml" -p roder-cli --bin roder -- app-server --listen "$(LISTEN)"

jaeger:
	./jaeger.sh

test:
	cargo test --workspace

smoke: test

release-brew:
	./scripts/release-brew.sh $(VERSION)

clean:
	rm -rf bin .gode
