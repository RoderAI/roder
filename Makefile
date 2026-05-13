BINARY ?= bin/gode
WORKSPACE ?= .
DATA_DIR ?= .gode
PROVIDER ?= openai
MODEL ?= gpt-5.4-mini
REASONING ?= low
PROMPT ?= summarize this repo in one sentence
LISTEN ?= ws://127.0.0.1:0
TELEMETRY ?= true
TELEMETRY_ENDPOINT ?= localhost:4317

.PHONY: build run ask app-server mock-app-server mock-run mock-ask tui jaeger test smoke clean

build:
	@mkdir -p $(dir $(BINARY))
	go build -o $(BINARY) ./cmd/gode

run: build
	$(BINARY) --workspace "$(WORKSPACE)" --data-dir "$(DATA_DIR)" --provider "$(PROVIDER)" --model "$(MODEL)" --reasoning "$(REASONING)" --auto-approve --telemetry=$(TELEMETRY) --telemetry-endpoint "$(TELEMETRY_ENDPOINT)"

ask: build
	$(BINARY) run --workspace "$(WORKSPACE)" --data-dir "$(DATA_DIR)" --provider "$(PROVIDER)" --model "$(MODEL)" --reasoning "$(REASONING)" --auto-approve "$(PROMPT)"

app-server: build
	$(BINARY) app-server --listen "$(LISTEN)" --workspace "$(WORKSPACE)" --data-dir "$(DATA_DIR)" --provider "$(PROVIDER)" --model "$(MODEL)" --reasoning "$(REASONING)" --auto-approve

mock-run: build
	$(BINARY) --workspace "$(WORKSPACE)" --data-dir "$(DATA_DIR)" --provider "mock" --model "$(MODEL)" --reasoning "$(REASONING)" --auto-approve

mock-ask: build
	$(BINARY) run --workspace "$(WORKSPACE)" --data-dir "$(DATA_DIR)" --provider "mock" --model "$(MODEL)" --reasoning "$(REASONING)" --auto-approve "$(PROMPT)"

mock-app-server: build
	$(BINARY) app-server --listen "$(LISTEN)" --workspace "$(WORKSPACE)" --data-dir "$(DATA_DIR)" --provider "mock" --model "$(MODEL)" --reasoning "$(REASONING)" --auto-approve

tui: build
	$(BINARY) --workspace "$(WORKSPACE)" --data-dir "$(DATA_DIR)" --provider "$(PROVIDER)" --model "$(MODEL)" --reasoning "$(REASONING)" --auto-approve

jaeger:
	./jaeger.sh

test:
	go test ./...

smoke: test mock-ask

clean:
	rm -rf bin .gode
