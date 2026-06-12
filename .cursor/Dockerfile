FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    clang \
    cmake \
    curl \
    git \
    lld \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Rust toolchain (version also pinned via rust-toolchain.toml in the repo).
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain 1.95.0 --profile minimal

ENV PATH="/root/.cargo/bin:${PATH}"
