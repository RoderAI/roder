BINARY ?= bin/gode
WORKSPACE ?= .
DATA_DIR ?=
PROVIDER ?=
MODEL ?=
REASONING ?=
PROMPT ?= summarize this repo in one sentence
LISTEN ?= ws://127.0.0.1:0
TELEMETRY ?= true
TELEMETRY_ENDPOINT ?= localhost:4317

WORKSPACE_FLAG = $(if $(WORKSPACE),--workspace "$(WORKSPACE)")
DATA_DIR_FLAG = $(if $(DATA_DIR),--data-dir "$(DATA_DIR)")
PROVIDER_FLAG = $(if $(PROVIDER),--provider "$(PROVIDER)")
MODEL_FLAG = $(if $(MODEL),--model "$(MODEL)")
REASONING_FLAG = $(if $(REASONING),--reasoning "$(REASONING)")
CONFIG_FLAGS = $(WORKSPACE_FLAG) $(DATA_DIR_FLAG) $(PROVIDER_FLAG) $(MODEL_FLAG) $(REASONING_FLAG)

.PHONY: build run ask app-server mock-app-server mock-run mock-ask tui jaeger test smoke clean

build:
	@mkdir -p $(dir $(BINARY))
	go build -o $(BINARY) ./cmd/gode

run: build
	$(BINARY) $(CONFIG_FLAGS) --auto-approve --telemetry=$(TELEMETRY) --telemetry-endpoint "$(TELEMETRY_ENDPOINT)"

ask: build
	$(BINARY) run $(CONFIG_FLAGS) --auto-approve "$(PROMPT)"

app-server: build
	$(BINARY) app-server --listen "$(LISTEN)" $(CONFIG_FLAGS) --auto-approve

mock-run: build
	$(BINARY) $(WORKSPACE_FLAG) $(DATA_DIR_FLAG) --provider "mock" $(MODEL_FLAG) $(REASONING_FLAG) --auto-approve

mock-ask: build
	$(BINARY) run $(WORKSPACE_FLAG) $(DATA_DIR_FLAG) --provider "mock" $(MODEL_FLAG) $(REASONING_FLAG) --auto-approve "$(PROMPT)"

mock-app-server: build
	$(BINARY) app-server --listen "$(LISTEN)" $(WORKSPACE_FLAG) $(DATA_DIR_FLAG) --provider "mock" $(MODEL_FLAG) $(REASONING_FLAG) --auto-approve

tui: build
	$(BINARY) $(CONFIG_FLAGS) --auto-approve

jaeger:
	./jaeger.sh

test:
	go test ./...

smoke: test mock-ask

clean:
	rm -rf bin .gode
