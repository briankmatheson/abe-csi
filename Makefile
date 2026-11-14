# abe-csi — build, test, package, deploy.
CARGO      ?= cargo
ARCH       := $(shell dpkg --print-architecture)
VERSION    := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
REVISION   := 1
DEB        := target/debian/abe-csi_$(VERSION)-$(REVISION)_$(ARCH).deb

.PHONY: build test lint deb tag push

build:
	$(CARGO) build --release

test:
	$(CARGO) test
	tests/lint-links.sh

lint:
	$(CARGO) fmt -- --check
	$(CARGO) clippy --all-targets -- -D warnings

deb:
	@$(CARGO) deb --version >/dev/null 2>&1 || { echo "cargo-deb missing: cargo install cargo-deb"; exit 1; }
	$(CARGO) deb
	@ls -l $(DEB)

tag:
	@git diff --quiet || { echo "working tree not clean"; exit 1; }
	git tag -a v$(VERSION) -m "abe-csi v$(VERSION)"
	@echo "tagged v$(VERSION); now: make push"

push:
	git push --follow-tags
