// tonic::Status is the gRPC error type; every CSI handler returns it as Err.
#![allow(clippy::result_large_err)]

use std::path::Path;
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

mod csi {
    // generated from the upstream CSI spec; its doc comments are not ours to fix
    #![allow(clippy::doc_overindented_list_items, clippy::doc_lazy_continuation)]
    tonic::include_proto!("csi.v1");
}
mod controller;
mod exec;
mod identity;
mod k8s;
mod node;

use controller::ControllerService;
use identity::IdentityService;
use node::NodeService;

/// Socket path from `--endpoint=unix:///path` or `$CSI_ENDPOINT`, defaulting
/// to the path the manifests mount.
fn socket_path() -> String {
    let from_arg = std::env::args()
        .skip(1)
        .find_map(|a| a.strip_prefix("--endpoint=").map(str::to_string));
    let raw = from_arg
        .or_else(|| std::env::var("CSI_ENDPOINT").ok())
        .unwrap_or_else(|| "unix:///csi/csi.sock".into());
    raw.strip_prefix("unix://")
        .map(|p| if p.starts_with('/') { p.to_string() } else { format!("/{}", p) })
        .unwrap_or(raw)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let socket_path = socket_path();
    if Path::new(&socket_path).exists() {
        std::fs::remove_file(&socket_path)?;
    }
    if let Some(dir) = Path::new(&socket_path).parent() {
        std::fs::create_dir_all(dir)?;
    }

    let uds = UnixListener::bind(&socket_path)?;
    tracing::info!(
        "abe-csi {} ({}) listening on {}",
        env!("CARGO_PKG_VERSION"),
        identity::DRIVER_NAME,
        socket_path
    );

    Server::builder()
        .add_service(csi::identity_server::IdentityServer::new(IdentityService::default()))
        .add_service(csi::controller_server::ControllerServer::new(ControllerService))
        .add_service(csi::node_server::NodeServer::new(NodeService::default()))
        .serve_with_incoming(UnixListenerStream::new(uds))
        .await?;

    Ok(())
}
