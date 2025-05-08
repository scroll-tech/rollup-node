FROM ubuntu:24.04 AS chef

RUN apt-get update -y && apt-get upgrade -y

# Install basic packages
RUN apt-get install build-essential curl wget git pkg-config -y
# Install dev-packages
RUN apt-get install libclang-dev libssl-dev llvm -y

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"
ENV CARGO_HOME=/root/.cargo
RUN cargo install cargo-chef --locked --version  0.1.71

FROM chef AS planner
WORKDIR /app
RUN --mount=target=. \
    cargo chef prepare --recipe-path /recipe.json

FROM chef AS builder
WORKDIR /app
COPY --from=planner /recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
RUN --mount=target=. \
    cargo build --release --target-dir=/app-target

# Release

FROM ubuntu:24.04

RUN apt-get update -y && \
    apt-get install -y --no-install-recommends \
    libssl3 \
    ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app-target/release/rollup-node /bin/

EXPOSE 30303 30303/udp 9001 8545 8546

ENTRYPOINT ["rollup-node"]

