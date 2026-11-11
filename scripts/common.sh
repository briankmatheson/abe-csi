#!/usr/bin/env bash
# Shared helpers for the abe-csi node/controller scripts.
set -euo pipefail

ABE_STATE_DIR=${ABE_STATE_DIR:-/var/lib/abe}
ABE_VOL_DIR="$ABE_STATE_DIR/volumes"
ABE_HOSTS_FILE=${ABE_HOSTS_FILE:-/etc/abe/hosts}

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

ensure_dirs() { mkdir -p "$ABE_VOL_DIR"; }

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

# The single ABE host to use: HOSTS/hosts param, else /etc/abe/hosts.
# Accepts a comma/space list for forward compatibility; uses the first.
resolve_host() {
  local raw=${ABE_PARAM_HOSTS:-${ABE_PARAM_HOST:-}}
  if [[ -z "$raw" && -f "$ABE_HOSTS_FILE" ]]; then raw=$(cat "$ABE_HOSTS_FILE"); fi
  raw=$(echo "$raw" | tr ',;' '  ' | awk '{print $1}')
  [[ -n "$raw" ]] || return 1
  echo "$raw"
}

persist_plan() {
  local id="$1"; shift
  mkdir -p "$ABE_VOL_DIR/$id"
  echo "$*" > "$ABE_VOL_DIR/$id/plan"
}
read_plan() { cat "$ABE_VOL_DIR/$1/plan"; }

# GET http://host/configure -> "host:port:id" (new volume)
# Optional second arg: requested size in bytes (ABE rounds up to MiB).
# ABE >= 0.3.1 also returns `perf` (a short fio write test of the new LV);
# just that object is kept as $ABE_VOL_DIR/<id>/perf for the CSI controller
# to put on the PersistentVolume as annotations.
# ABE runs fio for ABE_FIO_SECONDS before replying, so allow for it.
configure_host() {
  local host="$1" bytes="${2:-}" resp port id url made perf
  url="http://$host/configure"
  [[ -n "$bytes" && "$bytes" != 0 ]] && url="$url?bytes=$bytes"
  resp=$(curl --silent --fail --max-time 120 "$url") || return 1
  port=$(echo "$resp" | jq -r .port); id=$(echo "$resp" | jq -r .id)
  [[ -n "$port" && -n "$id" && "$port" != null && "$id" != null ]] || return 1
  mkdir -p "$ABE_VOL_DIR/$id"
  # ABE >= 0.3.2 reports the LV size it made (request rounded up to a MiB);
  # the controller uses it as the PV capacity.
  made=$(echo "$resp" | jq -r '.bytes // empty')
  [[ -n "$made" ]] && echo "$made" > "$ABE_VOL_DIR/$id/bytes"
  perf=$(echo "$resp" | jq -c '.perf // empty')
  if [[ -n "$perf" ]]; then
    echo "$perf" > "$ABE_VOL_DIR/$id/perf"
    log "perf for $id: $(echo "$perf" | jq -r '"\(.write_mib_s) MiB/s, \(.write_iops) IOPS, \(.write_lat_ms) ms at bs=\(.bs)"')"
  fi
  echo "$host:$port:$id"
}

# GET http://host/id/<id> -> "host:port:id" (re-export existing volume;
# the port can change across ABE restarts)
# DELETE http://host/id/<id>: unexport and remove the LV. 404 (already gone)
# is treated as success.
delete_host() {
  local host="$1" id="$2" code
  code=$(curl --silent --output /dev/null --max-time 60 -w '%{http_code}' -X DELETE "http://$host/id/$id") || return 1
  [[ "$code" == 200 || "$code" == 404 ]]
}

refresh_host() {
  local host="$1" id="$2" resp port
  resp=$(curl --silent --fail --max-time 30 "http://$host/id/$id") || return 1
  port=$(echo "$resp" | jq -r .port)
  [[ -n "$port" && "$port" != null ]] || return 1
  echo "$host:$port:$id"
}

# Block device for an attached ABE volume. ABE labels the single GPT
# partition with the volume id; fall back to walking sysfs by NQN.
find_device() {
  local id="$1" dev ctrl
  dev=$(blkid -t "PARTLABEL=$id" -o device 2>/dev/null | head -n1 || true)
  if [[ -n "$dev" ]]; then echo "$dev"; return 0; fi
  for ctrl in /sys/class/nvme/nvme*; do
    [[ -r "$ctrl/subsysnqn" ]] || continue
    if [[ "$(cat "$ctrl/subsysnqn")" == "$id" ]]; then
      local ns
      ns=$(ls -d "$ctrl"/nvme*n1 2>/dev/null | head -n1 || true)
      [[ -n "$ns" ]] || continue
      ns=/dev/$(basename "$ns")
      if [[ -b "${ns}p1" ]]; then echo "${ns}p1"; else echo "$ns"; fi
      return 0
    fi
  done
  return 1
}

# Wait for the device to appear after nvme connect (udev + partition scan).
wait_device() {
  local id="$1" tries=${2:-20} dev
  for ((n=0; n<tries; n++)); do
    if dev=$(find_device "$id") && [[ -b "$dev" ]]; then echo "$dev"; return 0; fi
    sleep 0.5
  done
  return 1
}

# Load the NVMe/TCP initiator. Needs the host's /lib/modules mounted.
ensure_nvme_tcp() {
  [[ -e /dev/nvme-fabrics ]] && return 0
  modprobe nvme-tcp 2>/dev/null || modprobe nvme_tcp 2>/dev/null || true
  for _ in 1 2 3 4 5; do [[ -e /dev/nvme-fabrics ]] && return 0; sleep 0.2; done
  die "/dev/nvme-fabrics missing: nvme-tcp module not loaded (is /lib/modules mounted into the pod?)"
}

is_connected() {
  local id="$1" ctrl
  for ctrl in /sys/class/nvme/nvme*; do
    [[ -r "$ctrl/subsysnqn" ]] && [[ "$(cat "$ctrl/subsysnqn")" == "$id" ]] && return 0
  done
  return 1
}

mapper_name() { echo "abe-$1"; }
