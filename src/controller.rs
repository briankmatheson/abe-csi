use crate::csi as proto;
use crate::exec;
use crate::identity::DRIVER_NAME;
use crate::k8s;
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::Duration;
use tonic::{Request, Response, Status};

/// Where the scripts keep per-volume state (`$ABE_STATE_DIR/volumes/<id>/plan`).
fn state_dir() -> String {
    std::env::var("ABE_STATE_DIR").unwrap_or_else(|_| "/var/lib/abe".into())
}

/// What we remember about a provisioned volume so repeated CreateVolume
/// calls for the same name are idempotent (the provisioner retries freely)
/// and so the node side can reconstruct the NVMe-oF plan without sharing a
/// filesystem with us.
#[derive(Clone, Debug)]
struct Provisioned {
    volume_id: String,
    plan: String,
    capacity_bytes: i64,
}

/// The `perf` object in ABE's `/configure` reply: a short fio write test
/// run on the new LV before it was handed to us.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct Perf {
    pub bs: String,
    pub seconds: f64,
    pub write_mib_s: f64,
    pub write_iops: f64,
    pub write_lat_ms: f64,
}

impl Perf {
    /// Only the numbers, keyed under the driver name.
    pub fn annotations(&self) -> BTreeMap<String, String> {
        let k = |s: &str| format!("{}/{}", DRIVER_NAME, s);
        BTreeMap::from([
            (k("fio-bs"), self.bs.clone()),
            (k("fio-seconds"), fmt_num(self.seconds)),
            (k("write-mib-s"), fmt_num(self.write_mib_s)),
            (k("write-iops"), fmt_num(self.write_iops)),
            (k("write-lat-ms"), fmt_num(self.write_lat_ms)),
        ])
    }
}

/// Up to two decimals, no trailing zeros ("412", "77.3", "0.25").
fn fmt_num(v: f64) -> String {
    let s = format!("{:.2}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[derive(Default)]
pub struct ControllerService {
    by_name: Mutex<HashMap<String, Provisioned>>,
}

impl ControllerService {
    fn by_name_path(name: &str) -> String {
        format!("{}/volumes/by-name/{}", state_dir(), name)
    }

    fn remember(&self, name: &str, p: &Provisioned) {
        self.by_name
            .lock()
            .unwrap()
            .insert(name.to_string(), p.clone());
        let path = Self::by_name_path(name);
        if let Some(dir) = std::path::Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let body = format!("{}\n{}\n{}\n", p.volume_id, p.capacity_bytes, p.plan);
        if let Err(e) = std::fs::write(&path, body) {
            tracing::warn!("could not persist {}: {}", path, e);
        }
    }

    fn recall(&self, name: &str) -> Option<Provisioned> {
        if let Some(p) = self.by_name.lock().unwrap().get(name) {
            return Some(p.clone());
        }
        let body = std::fs::read_to_string(Self::by_name_path(name)).ok()?;
        let mut lines = body.lines();
        let volume_id = lines.next()?.to_string();
        let capacity_bytes = lines.next()?.parse().ok()?;
        let plan = lines.next().unwrap_or("").to_string();
        let p = Provisioned { volume_id, plan, capacity_bytes };
        self.by_name
            .lock()
            .unwrap()
            .insert(name.to_string(), p.clone());
        Some(p)
    }

    fn forget(&self, volume_id: &str) {
        let mut map = self.by_name.lock().unwrap();
        let names: Vec<String> = map
            .iter()
            .filter(|(_, p)| p.volume_id == volume_id)
            .map(|(n, _)| n.clone())
            .collect();
        for n in names {
            map.remove(&n);
            let _ = std::fs::remove_file(Self::by_name_path(&n));
        }
    }

    fn read_plan(volume_id: &str) -> Option<String> {
        std::fs::read_to_string(format!("{}/volumes/{}/plan", state_dir(), volume_id))
            .ok()
            .map(|s| s.trim().to_string())
    }

    /// Size of the LV ABE actually made (`volumes/<id>/bytes`, from the
    /// /configure reply): the request rounded up to a whole MiB.
    fn read_bytes(volume_id: &str) -> Option<i64> {
        std::fs::read_to_string(format!("{}/volumes/{}/bytes", state_dir(), volume_id))
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    /// ABE's fio characterization of a freshly created volume, saved by
    /// abe-create-volume as `$ABE_STATE_DIR/volumes/<id>/perf`.
    fn read_perf(volume_id: &str) -> Option<Perf> {
        let body = std::fs::read_to_string(format!("{}/volumes/{}/perf", state_dir(), volume_id)).ok()?;
        match serde_json::from_str::<Perf>(&body) {
            Ok(p) => Some(p),
            Err(e) => {
                tracing::warn!("perf for {} is not parseable ({}): {}", volume_id, e, body.trim());
                None
            }
        }
    }

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

    fn volume_response(name: &str, p: &Provisioned) -> proto::CreateVolumeResponse {
        let mut ctx = HashMap::new();
        ctx.insert("name".to_string(), name.to_string());
        if !p.plan.is_empty() {
            ctx.insert("PLAN".to_string(), p.plan.clone());
        }
        proto::CreateVolumeResponse {
            volume: Some(proto::Volume {
                capacity_bytes: p.capacity_bytes,
                volume_id: p.volume_id.clone(),
                volume_context: ctx,
                content_source: None,
                accessible_topology: vec![],
            }),
        }
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

        // Idempotent replay. Re-arm the annotator too: the PV may not have
        // existed when the first attempt's annotator ran.
        if let Some(existing) = self.recall(&req.name) {
            tracing::info!(
                "CreateVolume '{}' already provisioned as {}",
                req.name,
                existing.volume_id
            );
            if let Some(perf) = Self::read_perf(&existing.volume_id) {
                Self::annotate_pv(&req.name, &existing.volume_id, &perf);
            }
            return Ok(Response::new(Self::volume_response(&req.name, &existing)));
        }

        tracing::info!(
            "CreateVolume '{}' ({} bytes) parameters={:?}",
            req.name,
            capacity_bytes,
            req.parameters
        );

        // abe-create-volume <name> [KEY=VALUE...]  -> prints the ABE volume id
        let mut args = vec![req.name.clone()];
        args.extend(exec::kv_args(&req.parameters));
        if capacity_bytes > 0 {
            args.push(format!("CAPACITY_BYTES={}", capacity_bytes));
        }
        let volume_id = exec::run("abe-create-volume", &args)?;
        if volume_id.is_empty() {
            return Err(Status::internal(
                "abe-create-volume did not print a volume id",
            ));
        }

        // Report what ABE made, which is at least what was asked for.
        let actual = Self::read_bytes(&volume_id).unwrap_or(capacity_bytes);
        if actual < capacity_bytes {
            return Err(Status::internal(format!(
                "ABE made {} a {} byte volume, less than the {} requested",
                volume_id, actual, capacity_bytes
            )));
        }
        let p = Provisioned {
            plan: Self::read_plan(&volume_id).unwrap_or_default(),
            volume_id,
            capacity_bytes: actual,
        };
        self.remember(&req.name, &p);

        tracing::info!(
            "CreateVolume '{}' -> {} ({} bytes)",
            req.name,
            p.volume_id,
            p.capacity_bytes
        );
        match Self::read_perf(&p.volume_id) {
            Some(perf) => Self::annotate_pv(&req.name, &p.volume_id, &perf),
            None => tracing::info!("no perf data for {} (fio off or missing on the ABE host)", p.volume_id),
        }
        Ok(Response::new(Self::volume_response(&req.name, &p)))
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
        exec::run("abe-delete-volume", &[req.volume_id.clone()])?;
        self.forget(&req.volume_id);
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
            r#"{"bs":"4M","seconds":5.0,"write_mib_s":412.5,"write_iops":103.12,"write_lat_ms":77.3}"#,
        )
        .unwrap();
        let a = perf.annotations();
        assert_eq!(a["abe.csi.briankmatheson/fio-bs"], "4M");
        assert_eq!(a["abe.csi.briankmatheson/fio-seconds"], "5");
        assert_eq!(a["abe.csi.briankmatheson/write-mib-s"], "412.5");
        assert_eq!(a["abe.csi.briankmatheson/write-iops"], "103.12");
        assert_eq!(a["abe.csi.briankmatheson/write-lat-ms"], "77.3");
        assert_eq!(a.len(), 5);
    }

    #[test]
    fn fmt_num_trims() {
        assert_eq!(fmt_num(412.0), "412");
        assert_eq!(fmt_num(0.254), "0.25");
        assert_eq!(fmt_num(100.10), "100.1");
    }
}
