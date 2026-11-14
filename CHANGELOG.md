# Changelog

All notable changes to abe-csi will be documented in this file.

## [0.3.5] - 2026-11-13

### Added
- `XDEBBIE_CA` env var (and `--xdebbie-ca` flag): PEM file or PEM text of the
  CA that signed xdebbied's API certificate, so the CSI driver trusts an
  https xdebbied without installing the cert system-wide.
- `abe-create-volume` helper script.

### Fixed
- A full ABE (pool at capacity) is now reported as `ResourceExhausted`
  instead of an internal error, so Kubernetes retries on a different node.

## [0.3.4] - 2026-11-12

### Added
- Initial release: NVMe/TCP CSI driver backed by ABE volumes attached through
  xdebbied as hot-plugged virtio disks.
