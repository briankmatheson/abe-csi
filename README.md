# abe-csi

CSI driver for [ABE](../abe) NVMe-over-TCP volumes.

```
PVC ──csi-provisioner──> CreateVolume ──> abe-create-volume ──GET /configure──> ABE host
                                                  │                                (lvcreate, nvmet export)
                                                  └─ volume_context.PLAN = host:port:id

Pod ──kubelet──> NodePublishVolume(secrets) ──> abe-connect-and-mount
                                                  ├─ GET /id/<id>  (re-export; port may change)
                                                  ├─ nvme connect -t tcp
                                                  ├─ cryptsetup luksFormat/open   (if passphrase given)
                                                  ├─ mkfs.ext4                    (first use)
                                                  └─ mount
```

## Mode

Single ABE host, one NVMe namespace per volume, no RAID. `hosts` in the
StorageClass names the host (first entry used).

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

## Performance annotations

ABE (>= 0.3.1) runs a short `fio` write test on every volume it creates and
returns the numbers in its `/configure` reply. `abe-create-volume` keeps
just that `perf` object (`volumes/<id>/perf`), and the controller writes it
onto the PersistentVolume as annotations once the provisioner has created
it:

```
abe.csi.briankmatheson/fio-bs: "4M"
abe.csi.briankmatheson/fio-seconds: "5"
abe.csi.briankmatheson/write-mib-s: "412.5"
abe.csi.briankmatheson/write-iops: "103.12"
abe.csi.briankmatheson/write-lat-ms: "77.3"
```

```sh
kubectl get pv -o custom-columns='PV:.metadata.name,MiB/s:.metadata.annotations.abe\.csi\.briankmatheson/write-mib-s,IOPS:.metadata.annotations.abe\.csi\.briankmatheson/write-iops'
```

The PV is named after the CSI `CreateVolume` name, so the controller polls
for it (up to 10 minutes, backing off to 10s) and merge-patches only those
keys; it checks `spec.csi.volumeHandle` first. If the ABE host has no
`fio` or `ABE_FIO_SECONDS=0`, there is no `perf` and nothing is annotated.
The provisioner's `--timeout` is 120s to leave room for the fio run.

## Deploy

```sh
docker build -t registry.internal.timesink.harlem.brianmatheson.com/abe-csi:latest .
docker push    registry.internal.timesink.harlem.brianmatheson.com/abe-csi:latest
kubectl -n kube-system create secret generic abe-luks --from-literal=passphrase="$(openssl rand -base64 48)"
kubectl apply -f k8s/rbac.yaml -f k8s/csi-driver.yaml -f k8s/controller.yaml -f k8s/node.yaml -f k8s/storageclass.yaml
kubectl apply -f k8s/pvc.yaml -f k8s/debian-pod.yaml
```

## Known limits

- No volume expansion (`allowVolumeExpansion: false`).
- No `NodeStageVolume`: several pods on one node share the mount via
  repeated `NodePublishVolume`; the connection is dropped when the last one
  unpublishes.
