FROM rust:1-trixie AS builder
WORKDIR /usr/src/abe-csi
RUN apt-get update && apt-get install -y pkg-config libssl-dev protobuf-compiler libprotobuf-dev && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo clean && cargo build --release

# Same Debian release as the builder: a binary built against trixie glibc
# does not run on bookworm.
FROM debian:trixie-slim
# Install dependencies including nvme-cli (with TLS support in trixie) and tlshd for NVMe/TCP TLS
# Note: tlshd may need to be built from source or installed from backports if not in trixie
RUN apt-get update && apt-get install -y \
    bash \
    curl \
    jq \
    nvme-cli \
    mdadm \
    cryptsetup \
    parted \
    util-linux \
    e2fsprogs \
    kmod \
    && rm -rf /var/lib/apt/lists/*

# TODO: Install tlshd daemon for TLS handshake support
# This may require building from source or using a backport
# For now, add a placeholder config that will be used if tlshd is available
RUN mkdir -p /etc/tlshd
RUN mkdir -p /opt/bin /var/lib/abe
COPY --from=builder /usr/src/abe-csi/target/release/abe-csi-rs /opt/bin/abe-csi-rs
COPY scripts/* /opt/bin/
COPY config/tlshd.conf /etc/tlshd/tlshd.conf
RUN chmod +x /opt/bin/* && ls -l /opt/bin
EXPOSE 50051
ENTRYPOINT ["/opt/bin/abe-csi-rs"]
