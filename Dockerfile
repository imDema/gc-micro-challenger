# BUILDER
FROM rust:1.100-slim-bookworm AS builder

RUN apt-get update && apt-get install build-essential clang pkg-config libfreetype6-dev libfontconfig1-dev  -y 

WORKDIR /build

### PREBUILD AND CACHE DEPENDENCIES
COPY Cargo.toml Cargo.lock ./

### PREBUILD AND CACHE DEPENDENCIES 
RUN mkdir -p src && touch src/lib.rs && cargo build --release --lib && touch src/lib.rs

COPY . .

RUN  touch src/lib.rs src/main.rs && cargo build --release

WORKDIR /build/target/release
RUN mkdir -p /app/ && mv micro-challenger /app/

# RUNNER
FROM debian:bookworm-slim
RUN apt-get update && apt-get install libfreetype6 libfontconfig1 -y 

COPY --from=builder /app/ /app/
COPY --from=builder /build/html/ /app/html/
ENV PATH="/app:${PATH}"

WORKDIR /app/
EXPOSE 8866
ENTRYPOINT [ "/app/micro-challenger" ]