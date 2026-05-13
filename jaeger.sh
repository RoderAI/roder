#!/usr/bin/env bash
set -euo pipefail

name="${JAEGER_CONTAINER:-gode-jaeger}"
image="${JAEGER_IMAGE:-jaegertracing/all-in-one:latest}"

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is required to start local Jaeger" >&2
  exit 1
fi

if docker ps --format '{{.Names}}' | grep -qx "$name"; then
  echo "Jaeger is already running in container $name"
elif docker ps -a --format '{{.Names}}' | grep -qx "$name"; then
  docker start "$name" >/dev/null
  echo "Started existing Jaeger container $name"
else
  docker run -d \
    --name "$name" \
    -e COLLECTOR_OTLP_ENABLED=true \
    -p 16686:16686 \
    -p 4317:4317 \
    -p 4318:4318 \
    "$image" >/dev/null
  echo "Started Jaeger container $name"
fi

echo "Jaeger UI: http://localhost:16686"
echo "OTLP/gRPC endpoint: localhost:4317"
echo "OTLP/HTTP endpoint: http://localhost:4318"
