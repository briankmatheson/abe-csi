# abe-csi

CSI driver for [ABE](../abe) volumes, attached through
[xdebbied](../xdebbie) as hot-plugged virtio disks.

The Kubernetes nodes are xdebbie VMs. ABE lives on the hypervisors'
WireGuard underlay, where guests have no route (and no business) — so the
driver never speaks NVMe. It asks xdebbied, which creates the volume on ABE,
`nvme connect`s it on the VM's host and `virsh attach-device`s it as a
virtio disk whose serial is the volume id. Measured against an in-guest
NVMe/TCP initiator this was also the faster path (3× on sequential reads,
6× lower random-write p99), because the TCP session stays on the host.

```
PVC ──csi-provisioner──> CreateVolume ──> abe-create-volume ──POST /v1/volumes──> xdebbied ──GET /configure──> ABE
                                                                                               (lvcreate, fio, nvmet)
Pod ──kubelet──> NodePublishVolume(secrets) ──> abe-attach-and-mount
                                                  ├─ POST /v1/volumes/<id>/attach {vm: <our SMBIOS uuid>}
                                                  │     xdebbied: nvme connect on the host, hot-plug virtio
                                                  ├─ wait for /dev/disk/by-id/virtio-<id[:20]>
                                                  ├─ cryptsetup luksFormat/open   (if passphrase given)
                                                  ├─ mkfs.ext4                    (first use)
                                                  └─ mount
                 NodeUnpublishVolume ──> abe-unmount-and-detach ── POST /v1/volumes/<id>/detach
                 DeleteVolume        ──> abe-delete-volume      ── DELETE /v1/volumes/<id>
```

The driver is stateless: xdebbied keys volumes by name (a replayed
`CreateVolume` gets the same volume), knows what is attached where, and
unplugs a volume by itself if its VM is deleted.

## Configure

`kube-system/abe-csi-xdebbie` (see `k8s/xdebbie-secret.example.yaml`)
holds `url` (xdebbied as seen from the VMs, normally the overlay gateway
`http://10.23.0.1:7443`) and `token` (`XDEBBIE_TOKEN` from
`/etc/xdebbie/xdebbie_env` on a hypervisor). Both pods take it as
`XDEBBIE_URL` / `XDEBBIE_TOKEN`.

StorageClass parameter `host` (optional) names the hypervisor whose ABE
should hold the volumes; the default is any healthy host with one.

## Size

The PVC request becomes `bytes` in `POST /v1/volumes`; ABE makes the LV
that size rounded up to a whole MiB and the driver reports what it made as
the PV capacity. No expansion.

## Encryption

Set `csi.storage.k8s.io/node-publish-secret-name/-namespace` on the
StorageClass to a Secret with a `passphrase` key (see
`k8s/luks-secret.example.yaml`). Kubelet passes the Secret to
`NodePublishVolume`; the driver pipes it to `cryptsetup --key-file=-` on
stdin. It is never put in argv, the environment, or logs. Volumes are
LUKS2, mapped as `/dev/mapper/abe-<id>`.

Omit the secret parameters for unencrypted volumes. A LUKS volume published
without a passphrase, or a non-LUKS volume with data published *with* one,
is refused rather than guessed at.

Transport security is the host's business: ABE is reachable only on the
hypervisors' WireGuard underlay.

## Performance annotations

ABE (>= 0.3.5) runs a short `fio` test on every volume it creates (random
direct writes, 4k with one in flight for 5 s by default); xdebbied relays
the numbers as `perf` in the volume record and the controller writes them
onto the PersistentVolume as annotations once the provisioner has created
it:

```
abe.csi/fio-bs: "4k"
abe.csi/fio-iodepth: "1"
abe.csi/fio-seconds: "5"
abe.csi/write-lat-us: "187.46"
abe.csi/write-iops: "5000"
abe.csi/write-mib-s: "19.53"
```

`write-lat-us` is the mean completion latency of one small synchronous
write on that pool when the volume was made, allocation included (the
volume is new): the figure for latency-sensitive placement.

```sh
kubectl get pv -o custom-columns='PV:.metadata.name,LAT-US:.metadata.annotations.abe\.csi/write-lat-us,IOPS:.metadata.annotations.abe\.csi/write-iops'
```

The PV is named after the CSI `CreateVolume` name, so the controller polls
for it (up to 10 minutes, backing off to 10s) and merge-patches only those
keys; it checks `spec.csi.volumeHandle` first. If the ABE host has no
`fio` or `ABE_FIO_SECONDS=0`, there is no `perf` and nothing is annotated.
The provisioner's `--timeout` is 120s to leave room for the fio run.

## Deploy

```sh
docker build -t registry.mesh.harlem.bmath.nyc/abe-csi:0.3.4 .
docker push    registry.mesh.harlem.bmath.nyc/abe-csi:0.3.4
kubectl -n kube-system create secret generic abe-csi-xdebbie \
  --from-literal=url=http://10.23.0.1:7443 --from-literal=token="$XDEBBIE_TOKEN"
kubectl -n kube-system create secret generic abe-luks --from-literal=passphrase="$(openssl rand -base64 48)"
kubectl apply -f k8s/rbac.yaml -f k8s/csi-driver.yaml -f k8s/controller.yaml -f k8s/node.yaml -f k8s/storageclass.yaml
kubectl apply -f k8s/pvc.yaml -f k8s/debian-pod.yaml
```

Needs xdebbied >= 0.9.0 (`/v1/volumes`). `xdctl volume ls` on the
hypervisor shows what the cluster has made and where it is plugged in.

## Known limits

- No volume expansion (`allowVolumeExpansion: false`).
- No `NodeStageVolume`: several pods on one node share the mount via
  repeated `NodePublishVolume`; the disk is unplugged when the last one
  unpublishes.
- Volumes attach on the VM's own hypervisor only (xdebbied runs `nvme` and
  `virsh` locally); the node pod must see the VM's `/dev` and
  `/sys/class/dmi/id`.
