# mTLS Design for abe-csi

## Overview

Add in-kernel TLS 1.3 support for NVMe/TCP connections using Linux KTLS (kernel 6.7+).

## Architecture

### Components

1. **Kernel TLS (.nvme keyring)**
   - PSK storage for NVMe connections
   - Managed via `nvme-cli` tools
   - Kernel-space authentication

2. **tlshd Daemon**
   - User-space TLS handshake negotiation
   - Configured via `/etc/tlshd.conf`
   - Must run on each node

3. **Key Distribution**
   - Kubernetes Secret per volume or global
   - Injected into node pods
   - Loaded into `.nvme` keyring

### Connection Flow

```
PVC created → CSI Controller → ABE /configure
                               ↓
                         (optional: /tls/key/<id>)
                               ↓
Pod scheduled → CSI Node → import TLS key to .nvme keyring
                         → nvme connect --tls
                         → tlshd negotiates TLS 1.3
                         → encrypted NVMe/TCP session
```

## Implementation

### 1. StorageClass Parameters

```yaml
parameters:
  hosts: "10.23.0.100"
  fsType: ext4
  # NEW: TLS configuration
  tls: "true"                    # enable TLS for connections
  tlsKeySecret: "abe-nvme-tls"   # optional: secret with PSK
  tlsKeySecretNamespace: "default"
```

### 2. TLS Secret Format

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: abe-nvme-tls
  namespace: default
type: Opaque
data:
  # Base64-encoded TLS PSK (generated via nvme gen-tls-key)
  psk: <base64-encoded-key>
  # Optional: identity for the key
  identity: <base64-encoded-identity>
```

### 3. Node Pod Changes

#### Dockerfile
- Ensure nvme-cli >= 2.8 (TLS support)
- Include tlshd binary
- Add /etc/tlshd.conf

#### DaemonSet
- Mount host's `/lib/modules` (already done)
- Run tlshd as sidecar or init process
- Volume mount for TLS secrets

### 4. Script Changes

#### scripts/common.sh

Add functions:
```bash
# Check if kernel supports NVMe TLS
check_nvme_tls_support() {
  local kernel_version
  kernel_version=$(uname -r | cut -d. -f1-2)
  if [[ $(echo "$kernel_version >= 6.7" | bc -l) -eq 0 ]]; then
    log "WARNING: Kernel $kernel_version < 6.7, NVMe TLS not supported"
    return 1
  fi
  if ! nvme gen-tls-key --help &>/dev/null; then
    log "WARNING: nvme-cli doesn't support TLS commands"
    return 1
  fi
  return 0
}

# Import TLS key into .nvme keyring
import_tls_key() {
  local id="$1" psk="$2" identity="${3:-$id}"
  log "importing TLS key for $id into .nvme keyring"
  # Generate or import the key
  echo "$psk" | nvme gen-tls-key --import --identity="$identity" --keyring=.nvme
  # Alternatively if we have a key file:
  # nvme gen-tls-key --import --keyfile=/path/to/key --identity="$id" --keyring=.nvme
}

# Check if TLS key exists for a volume
has_tls_key() {
  local id="$1"
  nvme check-tls-key --subsysnqn="$id" --keyring=.nvme &>/dev/null
}

# Remove TLS key from keyring
remove_tls_key() {
  local id="$1"
  # Key removal - implementation depends on nvme-cli version
  log "TLS key cleanup for $id (if needed)"
}
```

#### scripts/abe-connect-and-mount

Add TLS support:
```bash
# After parse_kv and before LUKS_KEY handling:

TLS_ENABLED=${ABE_PARAM_TLS:-false}
TLS_PSK=""
TLS_IDENTITY=""

if [[ "$TLS_ENABLED" == "true" ]]; then
  if ! check_nvme_tls_support; then
    die "TLS requested but not supported on this system"
  fi
  
  # Read TLS key from stdin (similar to LUKS key pattern)
  if [[ "${ABE_TLS_KEY_STDIN:-0}" == "1" ]]; then
    TLS_PSK=$(cat)
  fi
  
  # Import key if provided and not already present
  if [[ -n "$TLS_PSK" && ! has_tls_key "$VOL_ID" ]]; then
    import_tls_key "$VOL_ID" "$TLS_PSK" "${ABE_PARAM_TLS_IDENTITY:-$VOL_ID}"
  fi
fi

# ... existing code ...

# In the nvme connect section:
if ! is_connected "$VOL_ID"; then
  ensure_nvme_tcp
  PLAN=$(refresh_host "$HOST" "$VOL_ID") || die "ABE /id/$VOL_ID failed on $HOST"
  persist_plan "$VOL_ID" "$PLAN"
  PORT=$(echo "$PLAN" | cut -d: -f2)
  
  # Build nvme connect command
  CONNECT_CMD="nvme connect -a $HOST -t tcp -s $PORT -n $VOL_ID"
  if [[ "$TLS_ENABLED" == "true" ]]; then
    CONNECT_CMD="$CONNECT_CMD --tls"
  fi
  
  log "$CONNECT_CMD"
  eval "$CONNECT_CMD"
fi
```

### 5. Rust Code Changes

#### src/node.rs

Update `node_publish_volume` to pass TLS parameters:

```rust
// Extract TLS parameters from volume_context
let tls_enabled = req.volume_context.get("tls")
    .and_then(|v| v.parse::<bool>().ok())
    .unwrap_or(false);

// Build environment for script
let mut env_vars = vec![];
if tls_enabled {
    env_vars.push(("ABE_PARAM_TLS", "true"));
}

// If TLS secret is provided in secrets
if let Some(tls_psk) = req.secrets.get("tls-psk") {
    env_vars.push(("ABE_TLS_KEY_STDIN", "1"));
    // Pass PSK via stdin similar to LUKS passphrase
}
```

### 6. ABE Host Changes (separate project)

The ABE server needs to:
1. Support TLS on its NVMe targets
2. Optionally expose `/tls/key/<id>` endpoint to provide PSKs
3. Configure nvmet with TLS enabled

### 7. tlshd Configuration

Create `/etc/tlshd.conf` in node pods:

```conf
[authenticate]
# TLS 1.3 with PSK
keyrings=.nvme

[verify]
# Optional: peer verification settings
```

## Deployment

### 1. Prerequisites
- Kubernetes nodes running kernel 6.7+
- nvme-cli >= 2.8 with TLS support
- tlshd binary available

### 2. Build Updated Image
```bash
docker build -t abe-csi:mtls .
docker push abe-csi:mtls
```

### 3. Create TLS Secrets
```bash
# Generate a PSK
PSK=$(nvme gen-tls-key --output=- | base64 -w0)

kubectl create secret generic abe-nvme-tls \
  --from-literal=psk="$PSK" \
  --namespace=kube-system
```

### 4. Update StorageClass
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

## Testing

1. **Verify kernel support**: `uname -r` >= 6.7
2. **Test nvme-cli TLS**: `nvme gen-tls-key --help`
3. **Create test PVC** with TLS-enabled StorageClass
4. **Verify encrypted connection**: Check nvme subsystem shows TLS enabled
5. **Performance test**: Compare TLS vs non-TLS performance

## Fallback & Compatibility

- Non-TLS connections still work (tls: "false" or omitted)
- Graceful degradation if TLS not supported
- Warning logs if TLS requested but unavailable

## Security Considerations

1. **Key Storage**: PSKs in Kubernetes Secrets (encrypted at rest)
2. **Key Distribution**: Per-volume or shared PSK model
3. **Key Rotation**: Manual for now, automation later
4. **Stdin Handling**: Never log or expose PSKs in environment/args
5. **Cipher Suite**: TLS_AES_128_GCM_SHA256 (mandatory per spec)

## Future Enhancements

1. Certificate-based auth (instead of PSK)
2. Automatic key rotation
3. Per-volume unique keys from ABE server
4. Integration with cert-manager for key lifecycle
