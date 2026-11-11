# Quick Start: TLS for abe-csi

## 5-Minute Setup

### Prerequisites Check
```bash
# Check kernel version (need 6.7+)
uname -r

# Check nvme-cli TLS support
nvme gen-tls-key --help

# Or run comprehensive test
./scripts/test-tls.sh
```

### Setup (3 commands)

```bash
# 1. Generate keys and create secret
./scripts/setup-tls.sh

# 2. Apply TLS StorageClass
kubectl apply -f k8s/storageclass-tls.yaml

# 3. Create PVC
kubectl apply -f - <<EOF
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: my-tls-volume
spec:
  accessModes: [ReadWriteOnce]
  storageClassName: abe-tls
  resources:
    requests:
      storage: 10Gi
EOF
```

### Verify

```bash
# Check PVC
kubectl get pvc my-tls-volume

# Check connection (from node)
nvme list-subsys

# Check logs
kubectl logs -n kube-system -l app=abe-csi-node -c abe-csi | grep TLS
```

## Configuration Patterns

### Pattern 1: TLS Only
```yaml
# Secret
apiVersion: v1
kind: Secret
metadata:
  name: abe-tls
  namespace: kube-system
stringData:
  tls-psk: "<base64-psk>"

---
# StorageClass
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: abe-tls
provisioner: csi.abe.brianmatheson.com
parameters:
  hosts: "10.23.0.100"
  tls: "true"
  csi.storage.k8s.io/node-publish-secret-name: abe-tls
  csi.storage.k8s.io/node-publish-secret-namespace: kube-system
```

### Pattern 2: TLS + LUKS (Defense in Depth)
```yaml
# Secret
apiVersion: v1
kind: Secret
metadata:
  name: abe-tls-luks
  namespace: kube-system
stringData:
  tls-psk: "<base64-psk>"
  passphrase: "<luks-passphrase>"

---
# StorageClass
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: abe-secure
provisioner: csi.abe.brianmatheson.com
parameters:
  hosts: "10.23.0.100"
  tls: "true"
  csi.storage.k8s.io/node-publish-secret-name: abe-tls-luks
  csi.storage.k8s.io/node-publish-secret-namespace: kube-system
```

### Pattern 3: Node Affinity (Mixed Cluster)
```yaml
# Label TLS-capable nodes
kubectl label node node1 abe.csi.briankmatheson/tls=true
kubectl label node node2 abe.csi.briankmatheson/tls=true

---
# Pod with node selector
apiVersion: v1
kind: Pod
metadata:
  name: app
spec:
  nodeSelector:
    abe.csi.briankmatheson/tls: "true"
  volumes:
  - name: data
    persistentVolumeClaim:
      claimName: my-tls-volume
```

## Key Commands

### Generate Keys
```bash
# TLS PSK only
PSK=$(nvme gen-tls-key --output=- | base64 -w0)

# TLS + LUKS
PSK=$(nvme gen-tls-key --output=- | base64 -w0)
LUKS=$(openssl rand -base64 48)
```

### Create Secrets
```bash
# TLS only
kubectl create secret generic abe-tls \
  --from-literal=tls-psk="$PSK" \
  -n kube-system

# TLS + LUKS
kubectl create secret generic abe-secure \
  --from-literal=tls-psk="$PSK" \
  --from-literal=passphrase="$LUKS" \
  -n kube-system
```

### Debugging
```bash
# Check node capabilities
kubectl exec -n kube-system <abe-csi-node-pod> -- /opt/bin/test-tls.sh

# Check NVMe connections
kubectl exec -n kube-system <abe-csi-node-pod> -- nvme list-subsys

# View logs with TLS info
kubectl logs -n kube-system <abe-csi-node-pod> -c abe-csi | grep -i tls

# Check kernel keyring (on node)
keyctl list @u | grep nvme
```

## Common Issues

### "TLS requested but not supported"
**Cause**: Kernel < 6.7 or nvme-cli < 2.8
**Fix**: Upgrade kernel/nvme-cli or set `tls: "false"`

### "tlshd not running"
**Cause**: tlshd daemon not installed/started
**Fix**: Install and start tlshd, or it may work anyway (depends on kernel version)

### "failed to import TLS key"
**Cause**: Invalid PSK or keyring issues
**Fix**: Regenerate PSK, check secret format

### Connection works without TLS flag
**Cause**: ABE server doesn't have TLS enabled
**Fix**: Configure ABE for TLS, verify with `nvme list-subsys`

## Performance Tips

1. **Hardware AES**: Use nodes with AES-NI instructions
2. **Benchmark**: Test TLS vs non-TLS for your workload
3. **Latency**: TLS handshake is one-time per connection
4. **CPU**: Monitor CPU usage during I/O

## Security Checklist

- [ ] etcd encryption enabled for Kubernetes Secrets
- [ ] RBAC configured to restrict secret access
- [ ] Strong PSKs generated (use nvme gen-tls-key)
- [ ] Separate PSKs per environment (dev/staging/prod)
- [ ] Key rotation schedule defined
- [ ] Combine TLS with LUKS for defense in depth
- [ ] Audit logs enabled for secret access
- [ ] Network segmentation for storage traffic

## Upgrade Path

### From non-TLS to TLS
1. Create new TLS StorageClass (different name)
2. Create new PVCs with TLS class
3. Copy data from old to new volumes
4. Update pods to use new PVCs
5. Delete old PVCs

**Note**: Cannot enable TLS on existing volumes

### Rolling Upgrade Nodes
1. Drain node
2. Upgrade kernel to 6.7+
3. Upgrade nvme-cli to 2.8+
4. Install tlshd
5. Label node: `abe.csi.briankmatheson/tls=true`
6. Uncordon node
7. Repeat for other nodes

## Next Steps

- **Full Docs**: See [TLS_README.md](TLS_README.md)
- **Design**: See [MTLS_DESIGN.md](MTLS_DESIGN.md)
- **Changelog**: See [CHANGELOG_TLS.md](CHANGELOG_TLS.md)

## Support

Issues? Check:
1. Run `./scripts/test-tls.sh`
2. Check logs: `kubectl logs -n kube-system -l app=abe-csi-node`
3. Verify ABE server TLS configuration
4. Review [TLS_README.md](TLS_README.md) troubleshooting section
