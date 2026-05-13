BINARY ?= bin/gode
WORKSPACE ?= .
DATA_DIR ?= .gode
PROVIDER ?= mock
MODEL ?= gpt-5.4-mini
REASONING ?= low
PROMPT ?= summarize this repo in one sentence

.PHONY: build run tui test smoke clean

build:
	@mkdir -p $(dir $(BINARY))
	go build -o $(BINARY) ./cmd/gode

run: build
	$(BINARY) run --workspace "$(WORKSPACE)" --data-dir "$(DATA_DIR)" --provider "$(PROVIDER)" --model "$(MODEL)" --reasoning "$(REASONING)" --auto-approve "$(PROMPT)"

tui: build
	$(BINARY) --workspace "$(WORKSPACE)" --data-dir "$(DATA_DIR)" --provider "$(PROVIDER)" --model "$(MODEL)" --reasoning "$(REASONING)" --auto-approve

test:
	go test ./...

smoke: test run

clean:
	rm -rf bin .gode
