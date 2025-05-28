FROM rust:1.86.0 AS chef

RUN apt-get update -y && apt-get upgrade -y

# Install basic packages
RUN apt-get update && apt-get -y upgrade && apt-get install -y libclang-dev pkg-config
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

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app-target/release/rollup-node /bin/

EXPOSE 30303 30303/udp 9001 8545 8546

ENTRYPOINT ["rollup-node"]