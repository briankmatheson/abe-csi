#!/usr/bin/env bash
# Shared helpers for the abe-csi node/controller scripts.
#
# Everything goes through xdebbied (the hypervisor control plane): it makes
# volumes on ABE and hot-plugs them into the VM we are running in as virtio
# disks. This pod never speaks NVMe; it waits for /dev/disk/by-id/virtio-*.
set -euo pipefail

XDEBBIE_URL=${XDEBBIE_URL:-http://10.23.0.1:7443}
# XDEBBIE_TOKEN comes from the abe-csi-xdebbie Secret.

# libdevmapper waits for a udev cookie after creating a mapping; inside a
# container the host's udev never acknowledges it and cryptsetup hangs.
# Let libdm create the nodes itself (/dev is the host's, bind-mounted).
export DM_DISABLE_UDEV=1

log() { echo "[abe][$(date +%H:%M:%S)] $*" >&2; }
die() { log "$*"; exit 1; }

need_root() {
  if [[ ${EUID:-$(id -u)} -ne 0 ]]; then die "must run as root"; fi
}

require_cmds() {
  local missing=0
  for c in "$@"; do
    command -v "$c" >/dev/null 2>&1 || { log "missing dependency: $c"; missing=1; }
  done
  [[ $missing -eq 0 ]] || die "install dependencies then retry"
}

# Parse KEY=VALUE args into shell variables ABE_PARAM_<KEY>. Keys are
# upper-cased so StorageClass parameters may be written in lower case.
parse_kv() {
  local kv k v
  for kv in "$@"; do
    [[ "$kv" == *=* ]] || continue
    k=${kv%%=*}; v=${kv#*=}
    k=$(echo "$k" | tr '[:lower:]' '[:upper:]' | tr -c 'A-Z0-9_\n' '_')
    printf -v "ABE_PARAM_$k" '%s' "$v"
  done
}

# --- xdebbied -------------------------------------------------------------

# xd METHOD /path [json-body]  -> response body in XD_BODY, status in XD_CODE.
# Returns non-zero only when the request itself failed (no HTTP status).
# Globals, not stdout: `RESP=$(xd ...)` would run xd in a subshell and the
# caller would never see XD_CODE.
XD_CODE=000
XD_BODY=
xd() {
  local m="$1" path="$2" body="${3:-}" out
  local args=(--silent --show-error --max-time "${XD_TIMEOUT:-120}" -X "$m"
              -H "Accept: application/json" -w $'\n%{http_code}')
  [[ -n "${XDEBBIE_TOKEN:-}" ]] && args+=(-H "Authorization: Bearer $XDEBBIE_TOKEN")
  [[ -n "$body" ]] && args+=(-H "Content-Type: application/json" --data "$body")
  out=$(curl "${args[@]}" "$XDEBBIE_URL/v1$path") || { XD_CODE=000; XD_BODY=; return 1; }
  XD_CODE=${out##*$'\n'}
  XD_BODY=${out%$'\n'*}
}
xd_ok() { [[ "$XD_CODE" == 2* ]]; }
# "<code>: <error text>" for logs
xd_err() { echo "$XD_CODE: $(echo "${1:-}" | jq -r '.error // .' 2>/dev/null | head -c 300)"; }

# --- this VM ----------------------------------------------------------------

# xdebbied's id for the VM we run in (same uuid libvirt stamps into SMBIOS).
vm_uuid() {
  local u
  u=$(tr '[:upper:]' '[:lower:]' < /sys/class/dmi/id/product_uuid 2>/dev/null) || die "cannot read /sys/class/dmi/id/product_uuid (not an xdebbie VM?)"
  [[ "$u" =~ ^[0-9a-f-]{36}$ ]] || die "unexpected product_uuid '$u'"
  echo "$u"
}

# Where the guest sees a hot-plugged volume: libvirt sets the virtio serial
# to the volume id, which virtio-blk truncates to 20 bytes.
guest_dev() { echo "/dev/disk/by-id/virtio-${1:0:20}"; }

# Wait for the virtio disk (udev link) to show up.
wait_device() {
  local id="$1" tries=${2:-30} dev
  dev=$(guest_dev "$id")
  for ((n=0; n<tries; n++)); do
    [[ -b "$dev" ]] && { echo "$dev"; return 0; }
    sleep 0.5
  done
  return 1
}

mapper_name() { echo "abe-$1"; }
