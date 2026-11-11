# NVMe/TCP TLS Support in abe-csi

## Overview

abe-csi now supports **in-kernel TLS 1.3** for NVMe over TCP connections, providing encrypted storage traffic using Linux Kernel TLS (KTLS). This feature uses the NVMe/TCP TLS support introduced in Linux kernel 6.7.

## Features

- **Transport Layer Security**: Encrypt NVMe/TCP traffic with TLS 1.3
- **Pre-Shared Key (PSK) Authentication**: Use kernel keyring for secure key storage
- **Defense in Depth**: Combine with LUKS for both transport and at-rest encryption
- **Flexible Configuration**: Enable per-StorageClass via parameters
- **Backward Compatible**: Non-TLS connections continue to work

## Requirements

### Kubernetes Nodes

- **Linux Kernel**: 6.7 or higher (for NVMe/TCP TLS support)
- **nvme-cli**: Version 2.8 or higher (for TLS commands)
- **tlshd**: TLS handshake daemon for user-space negotiation
- **Kernel Modules**: `nvme-tcp` with TLS support enabled

### ABE Server

- Configure NVMe targets with TLS enabled
- Support TLS 1.3 with PSK authentication
- Use cipher suite: `TLS_AES_128_GCM_SHA256`

### Verify Support

Check if your nodes support NVMe/TCP TLS:

```bash
# Check kernel version
uname -r  # Should be >= 6.7

# Check nvme-cli TLS support
nvme gen-tls-key --help

# Verify nvme-tcp module
modprobe nvme-tcp
ls -l /dev/nvme-fabrics

# Check if tlshd is available
which tlshd
```

## Quick Start

### 1. Generate TLS PSK

```bash
# Generate a Pre-Shared Key for TLS
nvme gen-tls-key --output=- | base64 -w0
```

### 2. Create Kubernetes Secret

```bash
PSK=$(nvme gen-tls-key --output=- | base64 -w0)

kubectl create secret generic abe-nvme-tls \
  --from-literal=tls-psk="$PSK" \
  --namespace=kube-system
```

### 3. Create TLS-Enabled StorageClass

```yaml
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: abe-tls
provisioner: csi.abe.brianmatheson.com
parameters:
  hosts: "10.23.0.100"
  fsType: ext4
  tls: "true"
  csi.storage.k8s.io/node-publish-secret-name: abe-nvme-tls
  csi.storage.k8s.io/node-publish-secret-namespace: kube-system
```

### 4. Create PVC

```yaml
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: my-tls-volume
spec:
  accessModes:
  - ReadWriteOnce
  storageClassName: abe-tls
  resources:
    requests:
      storage: 10Gi
```

## Configuration Options

### StorageClass Parameters

| Parameter | Description | Default | Example |
|-----------|-------------|---------|---------|
| `tls` | Enable TLS for NVMe/TCP connections | `false` | `"true"` |
| `csi.storage.k8s.io/node-publish-secret-name` | Secret containing TLS PSK | - | `abe-nvme-tls` |
| `csi.storage.k8s.io/node-publish-secret-namespace` | Namespace of the secret | - | `kube-system` |

### Secret Keys

The node-publish secret can contain:

| Key | Purpose | Required |
|-----|---------|----------|
| `tls-psk` or `psk` | TLS Pre-Shared Key (base64) | For TLS |
| `passphrase` or `luks-key` | LUKS encryption passphrase | For LUKS |

## Advanced Configurations

### TLS + LUKS (Defense in Depth)

Combine transport encryption (TLS) with at-rest encryption (LUKS):

```bash
# Generate both keys
PSK=$(nvme gen-tls-key --output=- | base64 -w0)
LUKS_PASS=$(openssl rand -base64 48)

kubectl create secret generic abe-combined-secrets \
  --from-literal=tls-psk="$PSK" \
  --from-literal=passphrase="$LUKS_PASS" \
  --namespace=kube-system
```

```yaml
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: abe-tls-luks
provisioner: csi.abe.brianmatheson.com
parameters:
  hosts: "10.23.0.100"
  fsType: ext4
  tls: "true"
  csi.storage.k8s.io/node-publish-secret-name: abe-combined-secrets
  csi.storage.k8s.io/node-publish-secret-namespace: kube-system
```

### Node Selection for TLS

Label nodes that support TLS:

```bash
# Label nodes with kernel 6.7+
kubectl label node <node-name> abe.csi.briankmatheson/tls=true
```

Update DaemonSet with node selector:

```yaml
nodeSelector:
  abe.csi.briankmatheson/tls: "true"
```

## Architecture

### Connection Flow with TLS

```
1. Pod requests PVC with TLS-enabled StorageClass
2. CSI driver receives NodePublishVolume request
3. Driver extracts TLS PSK from node-publish secret
4. Script imports PSK into kernel .nvme keyring
5. nvme connect with --tls flag initiates connection
6. tlshd daemon performs TLS 1.3 handshake
7. Encrypted NVMe/TCP session established
8. Volume mounted and available to pod
```

### Security Model

1. **Key Storage**: PSKs stored in Kubernetes Secrets (encrypted at rest with etcd encryption)
2. **Key Distribution**: Secrets mounted only to authorized pods via RBAC
3. **Key Loading**: PSKs imported into kernel keyring (never exposed in logs/env)
4. **Transport Security**: TLS 1.3 with mandatory cipher: `TLS_AES_128_GCM_SHA256`
5. **At-Rest Security**: Optional LUKS encryption on top of TLS

## Troubleshooting

### Check Node Capabilities

```bash
# SSH to node
kubectl debug node/<node-name> -it --image=ubuntu

# Inside debug pod
uname -r  # Check kernel version
nvme gen-tls-key --help  # Check nvme-cli
pgrep tlshd  # Check if tlshd is running
```

### Verify TLS Connection

```bash
# After connecting, check NVMe subsystems
nvme list-subsys

# Look for TLS in the connection properties
cat /sys/class/nvme/nvme*/subsysnqn
```

### Common Issues

**"TLS requested but not supported"**
- Kernel version < 6.7
- Update kernel or disable TLS in StorageClass

**"tlshd not running"**
- Start tlshd daemon on nodes
- Or deploy as sidecar in node pods

**"failed to import TLS key"**
- Check secret exists and has correct keys
- Verify secret is in correct namespace
- Check RBAC permissions

**Connection fails with TLS enabled**
- Ensure ABE server has TLS enabled
- Verify matching cipher suite configuration
- Check tlshd logs for handshake errors

### Debug Logging

Enable verbose logging in scripts:

```bash
# In node pod
export ABE_DEBUG=1
kubectl logs -n kube-system <abe-csi-node-pod> -c abe-csi
```

Check nvme-cli verbose output:

```bash
nvme connect -a <host> -t tcp -s <port> -n <nqn> --tls -v
```

## Performance Considerations

- **Throughput**: TLS adds CPU overhead for encryption/decryption
- **Latency**: Minimal impact for sequential I/O, slight increase for random I/O
- **CPU Usage**: Hardware AES-NI acceleration recommended
- **Comparison**: Benchmark TLS vs non-TLS for your workload

### Benchmark Example

```bash
# Non-TLS StorageClass
kubectl apply -f pvc-no-tls.yaml
fio --name=test --filename=/mnt/test --size=1G --rw=write --bs=4M

# TLS StorageClass  
kubectl apply -f pvc-tls.yaml
fio --name=test --filename=/mnt/test --size=1G --rw=write --bs=4M
```

## Migration Guide

### From Non-TLS to TLS

1. **Create TLS-enabled StorageClass** (new name)
2. **Create new PVCs** using TLS StorageClass
3. **Migrate data** from old to new volumes
4. **Update pod specs** to use new PVCs
5. **Delete old PVCs** when migration complete

**Note**: Cannot enable TLS on existing volumes; must create new ones.

### Gradual Rollout

1. Deploy TLS-capable nodes with kernel 6.7+
2. Label nodes: `abe.csi.briankmatheson/tls=true`
3. Create TLS StorageClass with node affinity
4. New workloads use TLS, existing workloads unchanged
5. Migrate workloads progressively

## Examples

See the `k8s/` directory for complete examples:

- `tls-secret.example.yaml`: TLS PSK secret
- `combined-secret.example.yaml`: TLS + LUKS secret
- `storageclass-tls.yaml`: TLS-enabled StorageClass
- `node-tls.yaml`: DaemonSet with TLS support

## Security Best Practices

1. **Use Strong Keys**: Generate cryptographically secure PSKs
2. **Rotate Keys**: Implement regular key rotation schedule
3. **Separate Secrets**: Use different PSKs per environment
4. **Enable etcd Encryption**: Encrypt Kubernetes Secrets at rest
5. **RBAC**: Restrict secret access to CSI components only
6. **Defense in Depth**: Combine TLS with LUKS encryption
7. **Audit**: Monitor secret access and NVMe connections
8. **Network Segmentation**: Isolate storage network

## Future Enhancements

- [ ] Certificate-based authentication (in addition to PSK)
- [ ] Automatic key rotation with cert-manager integration
- [ ] Per-volume unique keys
- [ ] Key derivation from ABE server
- [ ] mTLS with mutual certificate validation
- [ ] Performance optimizations with hardware offload

## References

- [Linux Kernel TLS for NVMe/TCP](https://blogs.oracle.com/linux/tls-and-nvmetcp)
- [NVMe/TCP TLS in Linux 6.7](https://lwn.net/Articles/941139/)
- [nvme-cli TLS Commands](https://man.archlinux.org/man/nvme-tls-key.1.en)
- [Red Hat NVMe over Fabrics Guide](https://docs.redhat.com/en/documentation/red_hat_enterprise_linux/9/html/managing_storage_devices/configuring-nvme-over-fabrics-using-nvme-tcp_managing-storage-devices)

## Support

For issues specific to TLS support, please include:

- Kernel version (`uname -r`)
- nvme-cli version (`nvme version`)
- tlshd status (`systemctl status tlshd` or `pgrep tlshd`)
- Logs from abe-csi-node pod
- Output of `nvme list-subsys`
