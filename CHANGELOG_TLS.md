# Changelog - TLS Support

## [0.3.0] - 2026-11-11

### Added - NVMe/TCP TLS Support

#### Features
- **In-kernel TLS 1.3 encryption** for NVMe/TCP connections using Linux KTLS
- **PSK authentication** via .nvme kernel keyring
- **Defense in depth**: Combine TLS (transport) with LUKS (at-rest) encryption
- **Graceful fallback**: Automatic fallback to non-TLS when not supported
- **Backward compatible**: Existing non-TLS volumes continue to work unchanged

#### New StorageClass Parameters
- `tls: "true"` - Enable TLS for NVMe/TCP connections

#### New Secret Keys
- `tls-psk` - TLS Pre-Shared Key (base64-encoded)
- Can be combined with existing `passphrase` for LUKS in single secret

#### Scripts
- **setup-tls.sh**: Interactive setup script for generating keys and creating secrets
  - Generates TLS PSKs using `nvme gen-tls-key`
  - Creates Kubernetes secrets automatically
  - Supports both TLS-only and combined TLS+LUKS configurations
  
- **test-tls.sh**: Comprehensive capability testing script
  - Validates kernel version (6.7+)
  - Checks nvme-cli TLS support (2.8+)
  - Tests tlshd availability
  - Verifies TLS key generation
  - Color-coded results

#### Kubernetes Manifests
- `k8s/tls-secret.example.yaml` - Example TLS PSK secret
- `k8s/combined-secret.example.yaml` - Example TLS + LUKS secret
- `k8s/storageclass-tls.yaml` - TLS-enabled StorageClass examples
- `k8s/node-tls.yaml` - DaemonSet with TLS capability checking

#### Documentation
- `TLS_README.md` (332 lines) - Comprehensive TLS documentation
  - Quick start guide
  - Configuration reference
  - Architecture details
  - Troubleshooting guide
  - Performance considerations
  - Security best practices
  
- `MTLS_DESIGN.md` (282 lines) - Design document
  - Architecture and components
  - Implementation details
  - Security considerations
  
- `IMPLEMENTATION_SUMMARY.md` (339 lines) - Complete implementation summary

#### Code Changes
- Enhanced `scripts/common.sh` with TLS support functions
  - `check_nvme_tls_support()` - Validates prerequisites
  - `import_tls_key()` - Manages PSK in kernel keyring
  - `has_tls_key()` - Checks key existence
  - `check_tlshd()` - Verifies daemon status

- Updated `scripts/abe-connect-and-mount`
  - TLS configuration parsing
  - PSK handling via environment variable
  - Conditional `--tls` flag on nvme connect
  - Separated TLS and LUKS secret handling

- Extended `src/exec.rs`
  - New `run_with_secrets()` function
  - Support for both stdin (LUKS) and environment (TLS) secrets

- Modified `src/node.rs`
  - TLS PSK extraction from node-publish secrets
  - Dual secret handling (TLS + LUKS)
  - Enhanced logging

- Updated `Dockerfile`
  - Ensured nvme-cli with TLS support
  - Added tlshd configuration directory
  - Included tlshd.conf

### Requirements

#### Mandatory (for TLS)
- Linux kernel 6.7 or higher
- nvme-cli version 2.8 or higher

#### Recommended
- tlshd daemon for TLS handshake
- Hardware AES-NI for performance

#### ABE Server
- Must support TLS on NVMe targets
- TLS 1.3 with TLS_AES_128_GCM_SHA256 cipher

### Configuration Example

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

### Migration Guide

1. Deploy updated abe-csi version
2. Label TLS-capable nodes (kernel 6.7+)
3. Generate TLS PSKs: `./scripts/setup-tls.sh`
4. Create TLS-enabled StorageClass
5. New PVCs will use TLS
6. Existing volumes continue working unchanged

### Security Notes

- TLS PSKs stored in Kubernetes Secrets (enable etcd encryption)
- PSKs loaded into kernel .nvme keyring at connection time
- Never exposed in logs, argv, or environment (except process-local env)
- Combine with LUKS for defense in depth
- RBAC recommended for secret access control

### Performance Impact

- Minimal latency overhead for TLS handshake (one-time per connection)
- CPU overhead for encryption/decryption (hardware AES-NI recommended)
- Throughput depends on CPU capabilities
- Benchmark your specific workload

### Known Limitations

- Cannot enable TLS on existing volumes (must create new PVC)
- Per-volume unique keys not yet implemented (uses shared PSK)
- No automatic key rotation (manual management required)
- tlshd may require manual installation on some distributions

### Future Enhancements

- Certificate-based authentication
- Automatic key rotation
- Per-volume unique keys from ABE server
- Integration with cert-manager
- Hardware offload support

### Breaking Changes

None. This is a fully backward-compatible feature addition.

### Deprecations

None.

### Bug Fixes

None in this release (new feature).

### Testing

```bash
# Test node capabilities
./scripts/test-tls.sh

# Setup TLS
./scripts/setup-tls.sh

# Deploy and test
kubectl apply -f k8s/storageclass-tls.yaml
kubectl apply -f k8s/pvc.yaml
kubectl get pvc
```

### Contributors

- Implementation based on Linux Kernel TLS for NVMe/TCP (kernel 6.7+)
- References Oracle blog post on TLS and NVMe/TCP
- Uses nvme-cli TLS support (version 2.8+)

---

For complete documentation, see [TLS_README.md](TLS_README.md).
For design details, see [MTLS_DESIGN.md](MTLS_DESIGN.md).
For implementation details, see [IMPLEMENTATION_SUMMARY.md](IMPLEMENTATION_SUMMARY.md).
