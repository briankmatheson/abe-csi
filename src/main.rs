use std::net::SocketAddr;
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
    let addr: SocketAddr = "0.0.0.0:5051".parse()?;
    tracing::info!(%addr, "starting abe-csi-rs server");

    Server::builder()
        .add_service(csi::identity_server::IdentityServer::new(IdentityService::default()))
        .add_service(csi::controller_server::ControllerServer::new(ControllerService::default()))
        .add_service(csi::node_server::NodeServer::new(NodeService::default()))
        .serve(addr)
        .await?;

    Ok(())
}
