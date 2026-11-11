use crate::csi as proto;
use crate::exec;
use tonic::{Request, Response, Status};

pub struct NodeService {
    node_id: String,
}

impl Default for NodeService {
    fn default() -> Self {
        // Set from the downward API (spec.nodeName) in node.yaml; fall back
        // to the hostname, which with hostNetwork is the node name anyway.
        let node_id = std::env::var("NODE_ID")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "unknown".into());
        Self { node_id }
    }
}

fn unimplemented<T>(what: &str) -> Result<Response<T>, Status> {
    Err(Status::unimplemented(format!("{} is not supported by abe-csi", what)))
}

#[tonic::async_trait]
impl proto::node_server::Node for NodeService {
    async fn node_publish_volume(
        &self,
        request: Request<proto::NodePublishVolumeRequest>,
    ) -> Result<Response<proto::NodePublishVolumeResponse>, Status> {
        let req = request.into_inner();
        if req.volume_id.is_empty() || req.target_path.is_empty() {
            return Err(Status::invalid_argument(
                "volume_id and target_path are required",
            ));
        }
        tracing::info!(
            "NodePublishVolume {} -> {} readonly={} context={:?}",
            req.volume_id,
            req.target_path,
            req.readonly,
            req.volume_context
        );

        // abe-connect-and-mount <volume_id> <target_path> [KEY=VALUE...]
        // volume_context carries PLAN=... from CreateVolume so this node can
        // reconnect without sharing state with the controller pod.
        let mut args = vec![req.volume_id.clone(), req.target_path.clone()];
        args.extend(exec::kv_args(&req.volume_context));

        let mut mount_opts: Vec<String> = Vec::new();
        if let Some(cap) = req.volume_capability.as_ref() {
            if let Some(proto::volume_capability::AccessType::Mount(m)) = cap.access_type.as_ref() {
                mount_opts.extend(m.mount_flags.iter().cloned());
                if !m.fs_type.is_empty() {
                    args.push(format!("FS_TYPE={}", m.fs_type));
                }
            }
        }
        if req.readonly {
            mount_opts.push("ro".into());
        }
        if !mount_opts.is_empty() && !req.volume_context.contains_key("MOUNT_OPTS") {
            args.push(format!("MOUNT_OPTS={}", mount_opts.join(",")));
        }

        // TLS PSK from the node-publish secret (tls-psk, psk, or tls-key)
        // Passed via environment variable to avoid conflicts with LUKS stdin
        let tls_psk = ["tls-psk", "psk", "tls-key"]
            .iter()
            .find_map(|k| req.secrets.get(*k))
            .filter(|s| !s.is_empty())
            .map(String::as_str);
        if tls_psk.is_some() {
            tracing::info!("NodePublishVolume {}: TLS PSK supplied", req.volume_id);
        }
        
        // LUKS passphrase from the node-publish secret (StorageClass
        // csi.storage.k8s.io/node-publish-secret-name). Piped to the script's
        // stdin; absent => volume is used unencrypted.
        let passphrase = ["passphrase", "key", "luks-key"]
            .iter()
            .find_map(|k| req.secrets.get(*k))
            .filter(|s| !s.is_empty())
            .map(String::as_str);
        if passphrase.is_some() {
            tracing::info!("NodePublishVolume {}: LUKS passphrase supplied", req.volume_id);
        }

        exec::run_with_secrets("abe-connect-and-mount", &args, passphrase, tls_psk)?;
        tracing::info!("NodePublishVolume {} mounted", req.volume_id);
        Ok(Response::new(proto::NodePublishVolumeResponse {}))
    }

    async fn node_unpublish_volume(
        &self,
        request: Request<proto::NodeUnpublishVolumeRequest>,
    ) -> Result<Response<proto::NodeUnpublishVolumeResponse>, Status> {
        let req = request.into_inner();
        if req.volume_id.is_empty() || req.target_path.is_empty() {
            return Err(Status::invalid_argument(
                "volume_id and target_path are required",
            ));
        }
        tracing::info!("NodeUnpublishVolume {} from {}", req.volume_id, req.target_path);
        exec::run(
            "abe-unmount-and-disconnect",
            &[req.volume_id.clone(), req.target_path.clone()],
        )?;
        Ok(Response::new(proto::NodeUnpublishVolumeResponse {}))
    }

    async fn node_get_capabilities(
        &self,
        _request: Request<proto::NodeGetCapabilitiesRequest>,
    ) -> Result<Response<proto::NodeGetCapabilitiesResponse>, Status> {
        // No STAGE_UNSTAGE: kubelet goes straight to NodePublishVolume.
        Ok(Response::new(proto::NodeGetCapabilitiesResponse {
            capabilities: vec![],
        }))
    }

    async fn node_get_info(
        &self,
        _request: Request<proto::NodeGetInfoRequest>,
    ) -> Result<Response<proto::NodeGetInfoResponse>, Status> {
        Ok(Response::new(proto::NodeGetInfoResponse {
            node_id: self.node_id.clone(),
            max_volumes_per_node: 0,
            accessible_topology: None,
        }))
    }

    // --- Not supported; NodeGetCapabilities doesn't advertise them. ---

    async fn node_stage_volume(
        &self,
        _r: Request<proto::NodeStageVolumeRequest>,
    ) -> Result<Response<proto::NodeStageVolumeResponse>, Status> {
        unimplemented("NodeStageVolume")
    }
    async fn node_unstage_volume(
        &self,
        _r: Request<proto::NodeUnstageVolumeRequest>,
    ) -> Result<Response<proto::NodeUnstageVolumeResponse>, Status> {
        unimplemented("NodeUnstageVolume")
    }
    async fn node_get_volume_stats(
        &self,
        _r: Request<proto::NodeGetVolumeStatsRequest>,
    ) -> Result<Response<proto::NodeGetVolumeStatsResponse>, Status> {
        unimplemented("NodeGetVolumeStats")
    }
    async fn node_expand_volume(
        &self,
        _r: Request<proto::NodeExpandVolumeRequest>,
    ) -> Result<Response<proto::NodeExpandVolumeResponse>, Status> {
        unimplemented("NodeExpandVolume")
    }
}
