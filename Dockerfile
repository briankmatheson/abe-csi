# ---- Build stage ----
FROM rust:1.87-slim as builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev protobuf-compiler curl jq && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml .
COPY proto ./proto
RUN mkdir src && echo "fn main(){}" > src/main.rs
RUN cargo build --release || true
COPY src ./src
RUN cargo build --release

# ---- Runtime stage ----
FROM debian:latest
RUN apt-get update && apt-get install -y bash curl jq nvme-cli mdadm cryptsetup util-linux parted libssl3 strace && rm -rf /var/lib/apt/lists/*
RUN mkdir -p /usr/local/bin /var/lib/abe
COPY --from=builder /build/target/release/abe-csi-rs /usr/local/bin/abe-csi-rs
COPY scripts/* /usr/local/bin/
RUN chmod +x /usr/local/bin/*
EXPOSE 50051
ENTRYPOINT ["/bin/bash", "/usr/local/bin/entrypoint.sh"]
