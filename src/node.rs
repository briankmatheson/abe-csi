use crate::csi as proto;
use tonic::{Request, Response, Status};
use std::process::Command;

#[derive(Default)]
pub struct NodeService {}

#[tonic::async_trait]
impl proto::node_server::Node for NodeService {
    async fn node_publish_volume(
        &self,
        request: Request<proto::NodePublishVolumeRequest>,
    ) -> Result<Response<proto::NodePublishVolumeResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("NodePublishVolume called: volume_id={}, target_path={}, context={:?}", req.volume_id, req.target_path, req.volume_context);

        let mut cmd = Command::new("/usr/local/bin/abe-connect-and-mount");
        cmd.arg(&req.volume_id).arg(&req.target_path);
        for (k, v) in req.volume_context.clone() {
            cmd.arg(format!("{}={}", k, v));
        }

        let status = cmd.status().map_err(|e| Status::internal(format!("failed to exec connect script: {}", e)))?;
        if !status.success() {
            return Err(Status::internal("external connect script failed"));
        }

        tracing::info!("NodePublishVolume succeeded for '{}'", req.volume_id);
        Ok(Response::new(proto::NodePublishVolumeResponse {}))
    }

    async fn node_unpublish_volume(
        &self,
        request: Request<proto::NodeUnpublishVolumeRequest>,
    ) -> Result<Response<proto::NodeUnpublishVolumeResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("NodeUnpublishVolume called: volume_id={}, target_path={}", req.volume_id, req.target_path);

        let status = Command::new("/usr/local/bin/abe-unmount-and-disconnect")
            .arg(&req.volume_id)
            .arg(&req.target_path)
            .status()
            .map_err(|e| Status::internal(format!("failed to exec disconnect script: {}", e)))?;

        if !status.success() {
            return Err(Status::internal("external disconnect script failed"));
        }

        tracing::info!("NodeUnpublishVolume succeeded for '{}'", req.volume_id);
        Ok(Response::new(proto::NodeUnpublishVolumeResponse {}))
    }
}
