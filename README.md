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

## Deploy

```sh
docker build -t registry.internal.timesink.harlem.brianmatheson.com/abe-csi:latest .
docker push    registry.internal.timesink.harlem.brianmatheson.com/abe-csi:latest
kubectl -n kube-system create secret generic abe-luks --from-literal=passphrase="$(openssl rand -base64 48)"
kubectl apply -f k8s/rbac.yaml -f k8s/csi-driver.yaml -f k8s/controller.yaml -f k8s/node.yaml -f k8s/storageclass.yaml
kubectl apply -f k8s/pvc.yaml -f k8s/debian-pod.yaml
```

## Known limits

- ABE volumes are a fixed 100G (`VOL_SIZE` in abe); the PVC size is recorded
  but not enforced.
- ABE has no delete endpoint; `DeleteVolume` releases the client side and
  leaves the LV on the host.
- No `NodeStageVolume`: several pods on one node share the mount via
  repeated `NodePublishVolume`; the connection is dropped when the last one
  unpublishes.
