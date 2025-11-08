FROM rust:1.87-slim as builder
WORKDIR /usr/src/abe-csi
RUN apt-get update && apt-get install -y pkg-config libssl-dev protobuf-compiler curl jq && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo clean && cargo build --release

FROM debian:12-slim
RUN apt-get update && apt-get install -y bash curl jq nvme-cli mdadm cryptsetup parted && rm -rf /var/lib/apt/lists/*
RUN mkdir -p /opt/bin /var/lib/abe
COPY --from=builder /usr/src/abe-csi/target/release/abe-csi-rs /opt/bin/abe-csi-rs
COPY --from=builder /usr/src/abe-csi /usr/src/abe-csi
COPY scripts/* /opt/bin/
RUN chmod +x /opt/bin/* && ls -l /opt/bin
EXPOSE 50051
ENTRYPOINT ["/usr/src/abe-csi/target/release/abe-csi-rs"]
