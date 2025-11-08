FROM rust:1.87-slim as builder
WORKDIR /usr/src/abe-csi
RUN apt-get update && apt-get install -y pkg-config libssl-dev protobuf-compiler curl jq && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo build --release

FROM debian:12-slim
RUN apt-get update && apt-get install -y bash curl jq nvme-cli mdadm cryptsetup parted && rm -rf /var/lib/apt/lists/*
RUN mkdir -p /usr/local/bin /var/lib/abe
COPY --from=builder /usr/src/abe-csi/target/release/abe-csi-rs /usr/local/bin/abe-csi-rs
COPY scripts/* /usr/local/bin/
RUN chmod +x /usr/local/bin/* && ls -l /usr/local/bin
EXPOSE 50051
ENTRYPOINT ["/usr/local/bin/abe-csi-rs"]
