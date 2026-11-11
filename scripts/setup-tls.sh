#!/usr/bin/env bash
# Setup script for NVMe/TCP TLS in abe-csi
# Generates keys and creates Kubernetes secrets

set -euo pipefail

NAMESPACE="${NAMESPACE:-kube-system}"
SECRET_NAME="${SECRET_NAME:-abe-nvme-tls}"
COMBINED="${COMBINED:-false}"

usage() {
  cat <<EOF
Setup TLS for abe-csi NVMe/TCP connections

Usage: $0 [OPTIONS]

Options:
  -n, --namespace <ns>    Kubernetes namespace for secret (default: kube-system)
  -s, --secret <name>     Secret name (default: abe-nvme-tls)
  -c, --combined          Create combined secret with both TLS and LUKS keys
  -h, --help              Show this help message

Examples:
  # Create TLS-only secret
  $0

  # Create combined TLS + LUKS secret
  $0 --combined

  # Create in custom namespace
  $0 --namespace default --secret my-tls-secret

Requirements:
  - nvme-cli with TLS support (version 2.8+)
  - kubectl configured and connected to cluster
  - openssl (for LUKS passphrase generation if --combined)

EOF
  exit 0
}

# Parse arguments
while [[ $# -gt 0 ]]; do
  case $1 in
    -n|--namespace)
      NAMESPACE="$2"
      shift 2
      ;;
    -s|--secret)
      SECRET_NAME="$2"
      shift 2
      ;;
    -c|--combined)
      COMBINED=true
      shift
      ;;
    -h|--help)
      usage
      ;;
    *)
      echo "Unknown option: $1"
      usage
      ;;
  esac
done

echo "========================================="
echo "abe-csi TLS Setup"
echo "========================================="
echo "Namespace: $NAMESPACE"
echo "Secret:    $SECRET_NAME"
echo "Combined:  $COMBINED"
echo

# Check requirements
echo "Checking requirements..."

if ! command -v nvme &>/dev/null; then
  echo "ERROR: nvme command not found. Please install nvme-cli."
  exit 1
fi

if ! nvme gen-tls-key --help &>/dev/null; then
  echo "ERROR: nvme-cli doesn't support TLS commands."
  echo "Please install nvme-cli version 2.8 or higher."
  exit 1
fi

if ! command -v kubectl &>/dev/null; then
  echo "ERROR: kubectl not found. Please install kubectl."
  exit 1
fi

if [[ "$COMBINED" == "true" ]] && ! command -v openssl &>/dev/null; then
  echo "ERROR: openssl not found (needed for LUKS passphrase generation)."
  exit 1
fi

echo "✓ All requirements met"
echo

# Check if secret already exists
if kubectl get secret "$SECRET_NAME" -n "$NAMESPACE" &>/dev/null; then
  echo "WARNING: Secret $SECRET_NAME already exists in namespace $NAMESPACE"
  read -p "Do you want to replace it? (yes/no): " confirm
  if [[ "$confirm" != "yes" ]]; then
    echo "Aborted."
    exit 0
  fi
  kubectl delete secret "$SECRET_NAME" -n "$NAMESPACE"
fi

# Generate TLS PSK
echo "Generating TLS Pre-Shared Key..."
PSK_RAW=$(nvme gen-tls-key --output=-)
PSK_B64=$(echo -n "$PSK_RAW" | base64 -w0)

if [[ -z "$PSK_B64" ]]; then
  echo "ERROR: Failed to generate TLS PSK"
  exit 1
fi

echo "✓ TLS PSK generated"
echo

# Create secret
if [[ "$COMBINED" == "true" ]]; then
  echo "Generating LUKS passphrase..."
  LUKS_PASS=$(openssl rand -base64 48)
  echo "✓ LUKS passphrase generated"
  echo
  
  echo "Creating combined secret (TLS + LUKS)..."
  kubectl create secret generic "$SECRET_NAME" \
    --from-literal=tls-psk="$PSK_B64" \
    --from-literal=passphrase="$LUKS_PASS" \
    --namespace="$NAMESPACE"
  
  echo
  echo "========================================="
  echo "Combined Secret Created Successfully!"
  echo "========================================="
  echo "Namespace: $NAMESPACE"
  echo "Secret:    $SECRET_NAME"
  echo
  echo "Contains:"
  echo "  - tls-psk:    TLS Pre-Shared Key (base64)"
  echo "  - passphrase: LUKS encryption passphrase"
  echo
  echo "Use in StorageClass:"
  echo "  csi.storage.k8s.io/node-publish-secret-name: $SECRET_NAME"
  echo "  csi.storage.k8s.io/node-publish-secret-namespace: $NAMESPACE"
  echo
  echo "IMPORTANT: Save the LUKS passphrase securely!"
  echo "LUKS Passphrase: $LUKS_PASS"
  echo
else
  echo "Creating TLS secret..."
  kubectl create secret generic "$SECRET_NAME" \
    --from-literal=tls-psk="$PSK_B64" \
    --namespace="$NAMESPACE"
  
  echo
  echo "========================================="
  echo "TLS Secret Created Successfully!"
  echo "========================================="
  echo "Namespace: $NAMESPACE"
  echo "Secret:    $SECRET_NAME"
  echo
  echo "Use in StorageClass:"
  echo "  csi.storage.k8s.io/node-publish-secret-name: $SECRET_NAME"
  echo "  csi.storage.k8s.io/node-publish-secret-namespace: $NAMESPACE"
  echo
fi

# Display example StorageClass
cat <<EOF
Example StorageClass:
---
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: abe-tls
provisioner: csi.abe.brianmatheson.com
parameters:
  hosts: "10.23.0.100"
  fsType: ext4
  tls: "true"
  csi.storage.k8s.io/node-publish-secret-name: $SECRET_NAME
  csi.storage.k8s.io/node-publish-secret-namespace: $NAMESPACE
---
EOF

echo
echo "Next steps:"
echo "1. Ensure ABE server has TLS enabled for NVMe targets"
echo "2. Apply the StorageClass configuration"
echo "3. Create PVCs using the new StorageClass"
echo "4. Verify TLS connections with: nvme list-subsys"
echo
