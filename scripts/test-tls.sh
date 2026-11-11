#!/usr/bin/env bash
# Test script for NVMe/TCP TLS functionality
# Run on a Kubernetes node to verify TLS support

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

pass() { echo -e "${GREEN}✓${NC} $1"; }
fail() { echo -e "${RED}✗${NC} $1"; }
warn() { echo -e "${YELLOW}⚠${NC} $1"; }
info() { echo "ℹ $1"; }

echo "========================================="
echo "NVMe/TCP TLS Support Test"
echo "========================================="
echo

# Test 1: Kernel Version
info "Checking kernel version..."
KERNEL=$(uname -r)
MAJOR=$(echo "$KERNEL" | cut -d. -f1)
MINOR=$(echo "$KERNEL" | cut -d. -f2)

if [[ "$MAJOR" -gt 6 ]] || [[ "$MAJOR" -eq 6 && "$MINOR" -ge 7 ]]; then
  pass "Kernel version $KERNEL supports NVMe/TCP TLS (>= 6.7)"
  KERNEL_OK=true
else
  fail "Kernel version $KERNEL < 6.7 (TLS not supported)"
  warn "Upgrade kernel to 6.7+ for TLS support"
  KERNEL_OK=false
fi
echo

# Test 2: nvme-cli
info "Checking nvme-cli..."
if ! command -v nvme &>/dev/null; then
  fail "nvme command not found"
  warn "Install nvme-cli package"
  NVME_OK=false
else
  NVME_VERSION=$(nvme version 2>/dev/null | head -n1 || echo "unknown")
  if nvme gen-tls-key --help &>/dev/null 2>&1; then
    pass "nvme-cli supports TLS commands ($NVME_VERSION)"
    NVME_OK=true
  else
    fail "nvme-cli doesn't support TLS commands ($NVME_VERSION)"
    warn "Upgrade nvme-cli to version 2.8+"
    NVME_OK=false
  fi
fi
echo

# Test 3: nvme-tcp module
info "Checking nvme-tcp kernel module..."
if lsmod | grep -q nvme_tcp; then
  pass "nvme_tcp module is loaded"
  MODULE_OK=true
elif modprobe nvme-tcp 2>/dev/null || modprobe nvme_tcp 2>/dev/null; then
  pass "nvme_tcp module loaded successfully"
  MODULE_OK=true
else
  fail "Cannot load nvme-tcp module"
  warn "Ensure kernel is compiled with CONFIG_NVME_TCP=m"
  MODULE_OK=false
fi

if [[ "$MODULE_OK" == "true" ]]; then
  if [[ -e /dev/nvme-fabrics ]]; then
    pass "/dev/nvme-fabrics device exists"
  else
    fail "/dev/nvme-fabrics device not found"
    MODULE_OK=false
  fi
fi
echo

# Test 4: tlshd
info "Checking tlshd daemon..."
if command -v tlshd &>/dev/null; then
  pass "tlshd binary found at $(which tlshd)"
  TLSHD_INSTALLED=true
  
  if pgrep -x tlshd >/dev/null 2>&1; then
    pass "tlshd daemon is running (PID: $(pgrep -x tlshd))"
    TLSHD_RUNNING=true
  else
    warn "tlshd daemon is not running"
    info "Start with: systemctl start tlshd"
    TLSHD_RUNNING=false
  fi
else
  warn "tlshd binary not found"
  info "Install tlshd for TLS handshake support"
  TLSHD_INSTALLED=false
  TLSHD_RUNNING=false
fi
echo

# Test 5: Kernel keyring
info "Checking kernel keyring support..."
if command -v keyctl &>/dev/null; then
  pass "keyctl command available"
  
  # Try to describe the .nvme keyring
  if keyctl list @u | grep -q "\.nvme" || keyctl describe @u 2>/dev/null | grep -q "nvme"; then
    info "Keyring .nvme may be available"
  else
    info "Keyring .nvme not yet created (will be created on first use)"
  fi
  KEYRING_OK=true
else
  warn "keyctl command not found"
  info "Install keyutils package for keyring management"
  KEYRING_OK=false
fi
echo

# Test 6: Test TLS key generation
info "Testing TLS key generation..."
if [[ "$NVME_OK" == "true" ]]; then
  TEMP_KEY=$(mktemp)
  if nvme gen-tls-key --output="$TEMP_KEY" 2>/dev/null; then
    pass "Successfully generated test TLS key"
    KEY_SIZE=$(wc -c < "$TEMP_KEY")
    info "Generated key size: $KEY_SIZE bytes"
    rm -f "$TEMP_KEY"
    KEYGEN_OK=true
  else
    fail "Failed to generate TLS key"
    KEYGEN_OK=false
  fi
else
  warn "Skipping (nvme-cli doesn't support TLS)"
  KEYGEN_OK=false
fi
echo

# Summary
echo "========================================="
echo "Test Summary"
echo "========================================="

ALL_OK=true

if [[ "$KERNEL_OK" == "true" ]]; then
  pass "Kernel supports NVMe/TCP TLS"
else
  fail "Kernel does not support TLS"
  ALL_OK=false
fi

if [[ "$NVME_OK" == "true" ]]; then
  pass "nvme-cli has TLS support"
else
  fail "nvme-cli missing or lacks TLS support"
  ALL_OK=false
fi

if [[ "$MODULE_OK" == "true" ]]; then
  pass "nvme-tcp module is available"
else
  fail "nvme-tcp module issues"
  ALL_OK=false
fi

if [[ "$TLSHD_INSTALLED" == "true" ]]; then
  if [[ "$TLSHD_RUNNING" == "true" ]]; then
    pass "tlshd is installed and running"
  else
    warn "tlshd installed but not running"
  fi
else
  warn "tlshd not installed (recommended)"
fi

if [[ "$KEYGEN_OK" == "true" ]]; then
  pass "TLS key generation works"
else
  warn "Cannot generate TLS keys"
fi

echo
if [[ "$ALL_OK" == "true" ]]; then
  echo -e "${GREEN}=========================================${NC}"
  echo -e "${GREEN}✓ System is ready for NVMe/TCP TLS${NC}"
  echo -e "${GREEN}=========================================${NC}"
  echo
  echo "Next steps:"
  echo "1. Configure ABE server for TLS"
  echo "2. Run setup-tls.sh to create secrets"
  echo "3. Deploy TLS-enabled StorageClass"
  exit 0
else
  echo -e "${RED}=========================================${NC}"
  echo -e "${RED}✗ System is NOT ready for TLS${NC}"
  echo -e "${RED}=========================================${NC}"
  echo
  echo "Fix the failed checks above before using TLS."
  echo "TLS will not work without the required components."
  exit 1
fi
