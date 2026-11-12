use crate::csi as proto;
use tonic::{Request, Response, Status};

pub const DRIVER_NAME: &str = "abe.csi";

#[derive(Default)]
pub struct IdentityService {}

#[tonic::async_trait]
impl proto::identity_server::Identity for IdentityService {
    async fn get_plugin_info(
        &self,
        _request: Request<proto::GetPluginInfoRequest>,
    ) -> Result<Response<proto::GetPluginInfoResponse>, Status> {
        Ok(Response::new(proto::GetPluginInfoResponse {
            name: DRIVER_NAME.into(),
            vendor_version: env!("CARGO_PKG_VERSION").into(),
            manifest: Default::default(),
        }))
    }

    /// Sidecars use this to decide which services exist. We expose the
    /// controller service (provisioner needs it); no topology constraints.
    async fn get_plugin_capabilities(
        &self,
        _request: Request<proto::GetPluginCapabilitiesRequest>,
    ) -> Result<Response<proto::GetPluginCapabilitiesResponse>, Status> {
        use proto::plugin_capability::{service, Service, Type};
        Ok(Response::new(proto::GetPluginCapabilitiesResponse {
            capabilities: vec![proto::PluginCapability {
                r#type: Some(Type::Service(Service {
                    r#type: service::Type::ControllerService as i32,
                })),
            }],
        }))
    }

    /// Readiness. Every sidecar calls this before anything else; it was the
    /// missing RPC behind "CSI driver probe failed: Unimplemented".
    async fn probe(
        &self,
        _request: Request<proto::ProbeRequest>,
    ) -> Result<Response<proto::ProbeResponse>, Status> {
        Ok(Response::new(proto::ProbeResponse { ready: Some(true) }))
    }
}
