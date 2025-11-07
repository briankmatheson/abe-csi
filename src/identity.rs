use crate::csi as proto;
use tonic::{Request, Response, Status};

#[derive(Default)]
pub struct IdentityService {}

#[tonic::async_trait]
impl proto::identity_server::Identity for IdentityService {
    async fn get_plugin_info(
        &self,
        _request: Request<proto::GetPluginInfoRequest>,
    ) -> Result<Response<proto::GetPluginInfoResponse>, Status> {
        tracing::info!("GetPluginInfo called");
        Ok(Response::new(proto::GetPluginInfoResponse {
            name: "abe.csi.briankmatheson".into(),
            vendor_version: env!("CARGO_PKG_VERSION").into(),
        }))
    }
}
