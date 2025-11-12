use std::path::Path;
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

mod csi {
    tonic::include_proto!("csi");
}
mod identity;
mod controller;
mod node;

use identity::IdentityService;
use controller::ControllerService;
use node::NodeService;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let socket_path = "/csi/csi.sock";
    if Path::new(socket_path).exists() {
        std::fs::remove_file(socket_path)?;
    }

    let uds = UnixListener::bind(socket_path)?;
    tracing::info!("starting abe-csi-rs on {}", socket_path);

    Server::builder()
        .add_service(csi::identity_server::IdentityServer::new(IdentityService::default()))
        .add_service(csi::controller_server::ControllerServer::new(ControllerService::default()))
        .add_service(csi::node_server::NodeServer::new(NodeService::default()))
        .serve_with_incoming(UnixListenerStream::new(uds))
        .await?;

    Ok(())
}
