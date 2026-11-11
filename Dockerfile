FROM rust:1-trixie AS builder
WORKDIR /usr/src/abe-csi
RUN apt-get update && apt-get install -y pkg-config libssl-dev protobuf-compiler libprotobuf-dev && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo clean && cargo build --release

# Same Debian release as the builder: a binary built against trixie glibc
# does not run on bookworm.
FROM debian:trixie-slim
# No NVMe tooling: the hypervisor attaches volumes; this pod only sees virtio
# disks and needs the pieces to encrypt, format and mount them.
RUN apt-get update && apt-get install -y --no-install-recommends \
    bash \
    ca-certificates \
    curl \
    jq \
    cryptsetup-bin \
    util-linux \
    e2fsprogs \
    && rm -rf /var/lib/apt/lists/*

RUN mkdir -p /opt/bin
COPY --from=builder /usr/src/abe-csi/target/release/abe-csi-rs /opt/bin/abe-csi-rs
COPY scripts/* /opt/bin/
RUN chmod +x /opt/bin/*
ENTRYPOINT ["/opt/bin/abe-csi-rs"]
