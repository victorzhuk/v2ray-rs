# Tasks

## 1. CI Release Workflow and Linux Packaging Metadata

- [x] 1.1 Update the `appimage` job in `.github/workflows/release.yml` to resolve `VERSION` from `GITHUB_REF_NAME` on `refs/tags/v*` and from `[workspace.package].version` in `Cargo.toml` on `workflow_dispatch` (matching the `tarball` job), and verify the shell snippet extracts `X.Y.Z` cleanly on a non-tag ref
- [x] 1.2 Add `'protobuf'` to `makedepends` in `pkg/archlinux/PKGBUILD` and verify `PKGBUILD` syntax (`bash -n pkg/archlinux/PKGBUILD`) and version parity with `Cargo.toml`
- [x] 1.3 Fix `[package.metadata.deb]` in `crates/ui/Cargo.toml` so icon paths point to existing repository files (`crates/ui/icons/hicolor/scalable/apps/com.github.v2ray-rs.svg`), `v2ray-rs-netctl` and `v2ray-rs-run` are included with mode `755`, and runtime library dependencies are declared, verifying every referenced static path exists on disk

## 2. Makefile Workspace Coverage

- [x] 2.1 Add `NETCTL`, `RUN`, and `TUN_SESSION` variables plus `build-*` and `test-*` targets in `Makefile`, pass `--workspace` on `check`, `check-all`, `fix`, and `doc` targets, and verify `make -n check check-all build-netctl build-run build-tun-session test-tun-session` succeeds

## 3. Architecture, Release, and Contributor Documentation Parity

- [x] 3.1 Update `docs/ARCHITECTURE.md` to state `tray` depends only on `core`, document `crates/process/src/session/` (including `session/pins.rs`), update `crates/ui/src/` submodules and `WorkspaceStore`, and list `subscription_blobs/` under `$XDG_DATA_HOME/v2ray-rs/`, verifying all referenced paths exist
- [x] 3.2 Add `v2ray-rs.install` to the release artifact table in `docs/RELEASE.md` and update `CONTRIBUTING.md` (`Rust 1.93+`, `protobuf-compiler`, accurate bounded test commands, `persistence/` directory, `openspec/specs/<capability>/spec.md`, `.agents/skills/`), verifying all documented file paths exist in the repository
