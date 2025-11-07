use crate::csi as proto;
use tonic::{Request, Response, Status};
use std::process::Command;

#[derive(Default)]
pub struct ControllerService {}

#[tonic::async_trait]
impl proto::controller_server::Controller for ControllerService {
    async fn create_volume(
        &self,
        request: Request<proto::CreateVolumeRequest>,
    ) -> Result<Response<proto::CreateVolumeResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("CreateVolume called for '{}', parameters: {:?}", req.name, req.parameters);

        let mut cmd = Command::new("/usr/local/bin/abe-create-volume");
        cmd.arg(&req.name);
        for (k, v) in req.parameters.clone() {
            cmd.arg(format!("{}={}", k, v));
        }

        let status = cmd.status().map_err(|e| Status::internal(format!("failed to exec create script: {}", e)))?;
        if !status.success() {
            return Err(Status::internal("external create script failed"));
        }

        tracing::info!("CreateVolume succeeded for '{}'", req.name);
        Ok(Response::new(proto::CreateVolumeResponse { volume_id: req.name }))
    }

    async fn delete_volume(
        &self,
        request: Request<proto::DeleteVolumeRequest>,
    ) -> Result<Response<proto::DeleteVolumeResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("DeleteVolume called for '{}'", req.volume_id);

        let status = Command::new("/usr/local/bin/abe-delete-volume")
            .arg(&req.volume_id)
            .status()
            .map_err(|e| Status::internal(format!("failed to exec delete script: {}", e)))?;

        if !status.success() {
            return Err(Status::internal("external delete script failed"));
        }

        tracing::info!("DeleteVolume succeeded for '{}'", req.volume_id);
        Ok(Response::new(proto::DeleteVolumeResponse {}))
    }
}
