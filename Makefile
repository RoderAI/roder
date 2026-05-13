BINARY ?= bin/gode
WORKSPACE ?= .
DATA_DIR ?= .gode
PROVIDER ?= openai
MODEL ?= gpt-5.4-mini
REASONING ?= low
PROMPT ?= summarize this repo in one sentence
TELEMETRY ?= true
TELEMETRY_ENDPOINT ?= localhost:4317

.PHONY: build run ask mock-run mock-ask tui jaeger test smoke clean

build:
	@mkdir -p $(dir $(BINARY))
	go build -o $(BINARY) ./cmd/gode

run: build
	$(BINARY) --workspace "$(WORKSPACE)" --data-dir "$(DATA_DIR)" --provider "$(PROVIDER)" --model "$(MODEL)" --reasoning "$(REASONING)" --auto-approve --telemetry=$(TELEMETRY) --telemetry-endpoint "$(TELEMETRY_ENDPOINT)"

ask: build
	$(BINARY) run --workspace "$(WORKSPACE)" --data-dir "$(DATA_DIR)" --provider "$(PROVIDER)" --model "$(MODEL)" --reasoning "$(REASONING)" --auto-approve "$(PROMPT)"

mock-run: build
	$(BINARY) --workspace "$(WORKSPACE)" --data-dir "$(DATA_DIR)" --provider "mock" --model "$(MODEL)" --reasoning "$(REASONING)" --auto-approve

mock-ask: build
	$(BINARY) run --workspace "$(WORKSPACE)" --data-dir "$(DATA_DIR)" --provider "mock" --model "$(MODEL)" --reasoning "$(REASONING)" --auto-approve "$(PROMPT)"

tui: build
	$(BINARY) --workspace "$(WORKSPACE)" --data-dir "$(DATA_DIR)" --provider "$(PROVIDER)" --model "$(MODEL)" --reasoning "$(REASONING)" --auto-approve

jaeger:
	./jaeger.sh

test:
	go test ./...

smoke: test mock-ask

clean:
	rm -rf bin .gode
