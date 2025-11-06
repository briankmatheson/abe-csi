#!/usr/bin/env bash
set -euo pipefail

ABE_STATE_DIR=${ABE_STATE_DIR:-/var/lib/abe}
ABE_VOL_DIR="$ABE_STATE_DIR/volumes"
ABE_HOSTS_FILE=${ABE_HOSTS_FILE:-/etc/abe/hosts}

log() { echo "[abe][$(date +%H:%M:%S)] $*" >&2; }

need_root() {
  if [[ ${EUID:-$(id -u)} -ne 0 ]]; then
    echo "must run as root" >&2
    exit 4
  fi
}

require_cmds() {
  local missing=0
  for c in "$@"; do
    if ! command -v "$c" >/dev/null 2>&1; then log "missing dependency: $c"; missing=1; fi
  done
  if [[ $missing -eq 1 ]]; then
    log "install dependencies then retry"; exit 1
  fi
}

ensure_dirs() {
  mkdir -p "$ABE_VOL_DIR"
}

resolve_hosts() {
  local arg_hosts=${1:-}
  if [[ -n "$arg_hosts" ]]; then
    echo "$arg_hosts" | tr ',;' ' '
    return 0
  fi
  if [[ -f "$ABE_HOSTS_FILE" ]]; then
    tr ',;' ' ' < "$ABE_HOSTS_FILE"
    return 0
  fi
  echo ""; return 1
}

persist_plan() {
  local id="$1"; shift
  local plan="$*"
  mkdir -p "$ABE_VOL_DIR/$id"
  echo "$plan" > "$ABE_VOL_DIR/$id/plan"
}

read_plan() {
  local id="$1"
  cat "$ABE_VOL_DIR/$id/plan"
}

configure_host() {
  local host="$1"
  local url="http://$host/configure"
  local resp
  resp=$(curl --silent --fail "$url")
  local port id
  port=$(echo "$resp" | jq -r .port)
  id=$(echo "$resp" | jq -r .id)
  if [[ -z "$port" || -z "$id" || "$port" == "null" || "$id" == "null" ]]; then
    echo ""; return 1
  fi
  echo "$host:$port:$id"
}

refresh_host() {
  local host="$1" id="$2"
  local url="http://$host/id/$id"
  local resp port
  resp=$(curl --silent --fail "$url")
  port=$(echo "$resp" | jq -r .port)
  echo "$host:$port:$id"
}

by_partlabel() {
  local pl="$1"
  /sbin/blkid -t "PARTLABEL=$pl" | awk -F: '{print $1}'
}
