# syntax=docker/dockerfile:1
FROM --platform=$BUILDPLATFORM rust:1.86.0 AS chef

# Install basic packages and cross-compilation dependencies
RUN apt-get update -y && apt-get upgrade -y
RUN apt-get update && apt-get -y upgrade && apt-get install -y \
    libclang-dev pkg-config \
    gcc-aarch64-linux-gnu \
    libc6-dev-arm64-cross

# Install cargo-chef
RUN cargo install cargo-chef --locked --version 0.1.71

# Set up cross-compilation environment
ARG TARGETPLATFORM
RUN case "$TARGETPLATFORM" in \
    "linux/arm64") \
        rustup target add aarch64-unknown-linux-gnu && \
        echo '[target.aarch64-unknown-linux-gnu]' >> $CARGO_HOME/config.toml && \
        echo 'linker = "aarch64-linux-gnu-gcc"' >> $CARGO_HOME/config.toml \
        ;; \
    "linux/amd64") \
        rustup target add x86_64-unknown-linux-gnu \
        ;; \
    *) echo "Unsupported platform: $TARGETPLATFORM" && exit 1 ;; \
    esac

FROM chef AS planner
WORKDIR /app
RUN --mount=target=. \
    cargo chef prepare --recipe-path /recipe.json

FROM chef AS builder
WORKDIR /app
COPY --from=planner /recipe.json recipe.json

# Build dependencies with correct target
ARG TARGETPLATFORM
RUN case "$TARGETPLATFORM" in \
    "linux/arm64") \
        cargo chef cook --release --target aarch64-unknown-linux-gnu --recipe-path recipe.json \
        ;; \
    "linux/amd64") \
        cargo chef cook --release --target x86_64-unknown-linux-gnu --recipe-path recipe.json \
        ;; \
    esac

# Build the application with correct target
RUN --mount=target=. \
    case "$TARGETPLATFORM" in \
    "linux/arm64") \
        cargo build --release --target aarch64-unknown-linux-gnu --target-dir=/app-target \
        ;; \
    "linux/amd64") \
        cargo build --release --target x86_64-unknown-linux-gnu --target-dir=/app-target \
        ;; \
    esac

# Release
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the correct binary based on target platform
ARG TARGETPLATFORM
COPY --from=builder /app-target/ /tmp/app-target/

RUN case "$TARGETPLATFORM" in \
    "linux/arm64") \
        cp /tmp/app-target/aarch64-unknown-linux-gnu/release/rollup-node /bin/ \
        ;; \
    "linux/amd64") \
        cp /tmp/app-target/x86_64-unknown-linux-gnu/release/rollup-node /bin/ \
        ;; \
    esac

EXPOSE 30303 30303/udp 9001 8545 8546

ENTRYPOINT ["rollup-node"]

