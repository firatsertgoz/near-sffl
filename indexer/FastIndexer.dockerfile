FROM rust:1.79-bookworm AS builder
WORKDIR /tmp/indexer

# Copy from nearcore:
# https://github.com/near/nearcore/blob/master/Dockerfile
RUN apt-get update -qq && \
    apt-get install -y \
        git \
        cmake \
        g++ \
        pkg-config \
        libssl-dev \
        curl \
        llvm \
        clang

COPY ./Cargo.toml .
RUN mkdir ./src && echo "fn main() {}" > ./src/main.rs
RUN cargo build --release -p indexer --features use_fastnear

COPY . .
RUN touch ./src/main.rs

RUN cargo build --release -p indexer --features use_fastnear

FROM debian:bookworm-slim as runtime
WORKDIR /indexer-app

RUN apt update && apt install -yy openssl ca-certificates jq curl

COPY --from=builder /tmp/indexer/target/release/indexer .
COPY entrypoint.sh ./entrypoint.sh
RUN chmod +x ./entrypoint.sh

EXPOSE 3030

HEALTHCHECK --interval=20s --timeout=30s --retries=10000 \
  CMD (curl -f -s -X POST -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"block","params":{"finality":"optimistic"},"id":"dontcare"}' \
    http://localhost:3030 | \
  jq -es 'if . == [] then null else .[] | (now - (.result.header.timestamp / 1000000000)) < 10 end') && \
  (curl -f -s -X POST -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"status","params":[],"id":"dontcare"}' \
    http://localhost:3030 | \
  jq -es 'if . == [] then null else .[] | .result.sync_info.syncing == false end')

ENTRYPOINT [ "./entrypoint.sh" ]
