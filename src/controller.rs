use crate::csi as proto;
use crate::exec;
use crate::identity::DRIVER_NAME;
use crate::k8s;
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;
use tonic::{Request, Response, Status};

/// What `abe-create-volume` prints: xdebbied's Volume record.
#[derive(Clone, Debug, serde::Deserialize)]
struct Created {
    id: String,
    bytes: i64,
    #[serde(default)]
    perf: Option<Perf>,
}

/// The `perf` object ABE returned when it made the volume (a short fio
/// random-write test on the new LV: `bs` with `iodepth` in flight for
/// `seconds`), relayed by xdebbied.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct Perf {
    pub bs: String,
    pub iodepth: u32,
    pub seconds: f64,
    pub write_mib_s: f64,
    pub write_iops: f64,
    /// mean completion latency, microseconds
    pub write_lat_us: f64,
}

impl Perf {
    /// Only the numbers, keyed under the driver name.
    pub fn annotations(&self) -> BTreeMap<String, String> {
        let k = |s: &str| format!("{}/{}", DRIVER_NAME, s);
        BTreeMap::from([
            (k("fio-bs"), self.bs.clone()),
            (k("fio-iodepth"), self.iodepth.to_string()),
            (k("fio-seconds"), fmt_num(self.seconds)),
            (k("write-mib-s"), fmt_num(self.write_mib_s)),
            (k("write-iops"), fmt_num(self.write_iops)),
            (k("write-lat-us"), fmt_num(self.write_lat_us)),
        ])
    }
}

/// Up to two decimals, no trailing zeros ("412", "77.3", "0.25").
fn fmt_num(v: f64) -> String {
    let s = format!("{:.2}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Stateless: xdebbied keys volumes by name, so a replayed CreateVolume
/// gets the same volume back from it.
#[derive(Default)]
pub struct ControllerService;

impl ControllerService {
    /// Put ABE's perf numbers on the PV as annotations. The PV is created by
    /// the provisioner *after* CreateVolume returns (named after the request),
    /// so this runs in the background and waits for it.
    fn annotate_pv(pv_name: &str, volume_id: &str, perf: &Perf) {
        let client = match k8s::Client::in_cluster() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("not annotating pv/{}: {}", pv_name, e);
                return;
            }
        };
        tokio::spawn(k8s::annotate_pv_when_created(
            client,
            pv_name.to_string(),
            volume_id.to_string(),
            perf.annotations(),
            Duration::from_secs(600),
        ));
    }
}

fn unimplemented<T>(what: &str) -> Result<Response<T>, Status> {
    Err(Status::unimplemented(format!("{} is not supported by abe-csi", what)))
}

#[tonic::async_trait]
impl proto::controller_server::Controller for ControllerService {
    async fn create_volume(
        &self,
        request: Request<proto::CreateVolumeRequest>,
    ) -> Result<Response<proto::CreateVolumeResponse>, Status> {
        let req = request.into_inner();
        if req.name.is_empty() {
            return Err(Status::invalid_argument("name is required"));
        }
        let capacity_bytes = req
            .capacity_range
            .as_ref()
            .map(|r| r.required_bytes)
            .unwrap_or(0);

        tracing::info!(
            "CreateVolume '{}' ({} bytes) parameters={:?}",
            req.name,
            capacity_bytes,
            req.parameters
        );

        // abe-create-volume <name> [KEY=VALUE...]  -> prints xdebbied's Volume
        let mut args = vec![req.name.clone()];
        args.extend(exec::kv_args(&req.parameters));
        if capacity_bytes > 0 {
            args.push(format!("CAPACITY_BYTES={}", capacity_bytes));
        }
        let out = exec::run("abe-create-volume", &args)?;
        let made: Created = serde_json::from_str(&out).map_err(|e| {
            Status::internal(format!("abe-create-volume printed something odd ({}): {}", e, out))
        })?;
        if made.bytes < capacity_bytes {
            return Err(Status::internal(format!(
                "{} is a {} byte volume, less than the {} requested",
                made.id, made.bytes, capacity_bytes
            )));
        }
        tracing::info!("CreateVolume '{}' -> {} ({} bytes)", req.name, made.id, made.bytes);
        // Re-armed on replays too: the PV may not have existed the first time.
        match &made.perf {
            Some(perf) => Self::annotate_pv(&req.name, &made.id, perf),
            None => tracing::info!("no perf data for {} (fio off or missing on the ABE host)", made.id),
        }
        let mut ctx = HashMap::new();
        ctx.insert("name".to_string(), req.name.clone());
        Ok(Response::new(proto::CreateVolumeResponse {
            volume: Some(proto::Volume {
                capacity_bytes: made.bytes,
                volume_id: made.id,
                volume_context: ctx,
                content_source: None,
                accessible_topology: vec![],
            }),
        }))
    }

    async fn delete_volume(
        &self,
        request: Request<proto::DeleteVolumeRequest>,
    ) -> Result<Response<proto::DeleteVolumeResponse>, Status> {
        let req = request.into_inner();
        if req.volume_id.is_empty() {
            return Err(Status::invalid_argument("volume_id is required"));
        }
        tracing::info!("DeleteVolume {}", req.volume_id);
        exec::run("abe-delete-volume", std::slice::from_ref(&req.volume_id))?;
        Ok(Response::new(proto::DeleteVolumeResponse {}))
    }

    async fn controller_get_capabilities(
        &self,
        _request: Request<proto::ControllerGetCapabilitiesRequest>,
    ) -> Result<Response<proto::ControllerGetCapabilitiesResponse>, Status> {
        use proto::controller_service_capability::{rpc, Rpc, Type};
        let cap = |t: rpc::Type| proto::ControllerServiceCapability {
            r#type: Some(Type::Rpc(Rpc { r#type: t as i32 })),
        };
        Ok(Response::new(proto::ControllerGetCapabilitiesResponse {
            capabilities: vec![cap(rpc::Type::CreateDeleteVolume)],
        }))
    }

    /// We only do single-node RWO mounts. Accept those, refuse the rest.
    async fn validate_volume_capabilities(
        &self,
        request: Request<proto::ValidateVolumeCapabilitiesRequest>,
    ) -> Result<Response<proto::ValidateVolumeCapabilitiesResponse>, Status> {
        use proto::volume_capability::access_mode::Mode;
        let req = request.into_inner();
        let ok = req.volume_capabilities.iter().all(|c| {
            matches!(
                c.access_mode.as_ref().map(|m| m.mode()),
                Some(Mode::SingleNodeWriter)
                    | Some(Mode::SingleNodeReaderOnly)
                    | Some(Mode::SingleNodeSingleWriter)
                    | Some(Mode::SingleNodeMultiWriter)
            )
        });
        Ok(Response::new(proto::ValidateVolumeCapabilitiesResponse {
            confirmed: if ok {
                Some(proto::validate_volume_capabilities_response::Confirmed {
                    volume_context: req.volume_context,
                    volume_capabilities: req.volume_capabilities,
                    parameters: req.parameters,
                    mutable_parameters: req.mutable_parameters,
                })
            } else {
                None
            },
            message: if ok {
                String::new()
            } else {
                "only single-node access modes are supported".into()
            },
        }))
    }

    // --- Not supported; capabilities above don't advertise them. ---

    async fn controller_publish_volume(
        &self,
        _r: Request<proto::ControllerPublishVolumeRequest>,
    ) -> Result<Response<proto::ControllerPublishVolumeResponse>, Status> {
        unimplemented("ControllerPublishVolume")
    }
    async fn controller_unpublish_volume(
        &self,
        _r: Request<proto::ControllerUnpublishVolumeRequest>,
    ) -> Result<Response<proto::ControllerUnpublishVolumeResponse>, Status> {
        unimplemented("ControllerUnpublishVolume")
    }
    async fn list_volumes(
        &self,
        _r: Request<proto::ListVolumesRequest>,
    ) -> Result<Response<proto::ListVolumesResponse>, Status> {
        unimplemented("ListVolumes")
    }
    async fn get_capacity(
        &self,
        _r: Request<proto::GetCapacityRequest>,
    ) -> Result<Response<proto::GetCapacityResponse>, Status> {
        unimplemented("GetCapacity")
    }
    async fn create_snapshot(
        &self,
        _r: Request<proto::CreateSnapshotRequest>,
    ) -> Result<Response<proto::CreateSnapshotResponse>, Status> {
        unimplemented("CreateSnapshot")
    }
    async fn delete_snapshot(
        &self,
        _r: Request<proto::DeleteSnapshotRequest>,
    ) -> Result<Response<proto::DeleteSnapshotResponse>, Status> {
        unimplemented("DeleteSnapshot")
    }
    async fn list_snapshots(
        &self,
        _r: Request<proto::ListSnapshotsRequest>,
    ) -> Result<Response<proto::ListSnapshotsResponse>, Status> {
        unimplemented("ListSnapshots")
    }
    async fn controller_expand_volume(
        &self,
        _r: Request<proto::ControllerExpandVolumeRequest>,
    ) -> Result<Response<proto::ControllerExpandVolumeResponse>, Status> {
        unimplemented("ControllerExpandVolume")
    }
    async fn controller_get_volume(
        &self,
        _r: Request<proto::ControllerGetVolumeRequest>,
    ) -> Result<Response<proto::ControllerGetVolumeResponse>, Status> {
        unimplemented("ControllerGetVolume")
    }
    async fn controller_modify_volume(
        &self,
        _r: Request<proto::ControllerModifyVolumeRequest>,
    ) -> Result<Response<proto::ControllerModifyVolumeResponse>, Status> {
        unimplemented("ControllerModifyVolume")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perf_becomes_number_only_annotations() {
        let perf: Perf = serde_json::from_str(
            r#"{"bs":"4k","iodepth":1,"seconds":5.0,"write_mib_s":19.53,"write_iops":5000.0,"write_lat_us":187.456}"#,
        )
        .unwrap();
        let a = perf.annotations();
        assert_eq!(a["abe.csi/fio-bs"], "4k");
        assert_eq!(a["abe.csi/fio-iodepth"], "1");
        assert_eq!(a["abe.csi/fio-seconds"], "5");
        assert_eq!(a["abe.csi/write-mib-s"], "19.53");
        assert_eq!(a["abe.csi/write-iops"], "5000");
        assert_eq!(a["abe.csi/write-lat-us"], "187.46");
        assert_eq!(a.len(), 6);
    }

    #[test]
    fn created_parses_xdebbied_volume_record() {
        let c: Created = serde_json::from_str(
            r#"{"id":"7c0a1b2c3d4e5f60718293a4b5c6d7e8","name":"pvc-x","server":"http://10.22.0.1:80",
                "traddr":"10.22.0.1","port":"44002","bytes":5368709120,
                "perf":{"bs":"4k","iodepth":1,"seconds":5.0,"write_mib_s":19.53,"write_iops":5000.0,"write_lat_us":187.46},
                "created":"2026-11-11T00:00:00Z","updated":"2026-11-11T00:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(c.id, "7c0a1b2c3d4e5f60718293a4b5c6d7e8");
        assert_eq!(c.bytes, 5 << 30);
        assert_eq!(c.perf.unwrap().write_lat_us, 187.46);
        let no_perf: Created = serde_json::from_str(r#"{"id":"a","bytes":1}"#).unwrap();
        assert!(no_perf.perf.is_none());
    }

    #[test]
    fn fmt_num_trims() {
        assert_eq!(fmt_num(412.0), "412");
        assert_eq!(fmt_num(0.254), "0.25");
        assert_eq!(fmt_num(100.10), "100.1");
    }
}
