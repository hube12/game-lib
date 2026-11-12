FROM rust:latest AS builder

WORKDIR /build

RUN apt update && apt install -y musl-tools musl-dev
RUN update-ca-certificates

# Install the pinned toolchain and the musl target first so this layer is cached.
COPY rust-toolchain.toml .
RUN rustup show active-toolchain || rustup toolchain install
RUN rustup target add x86_64-unknown-linux-musl

COPY . .
RUN cargo build --target x86_64-unknown-linux-musl --release
