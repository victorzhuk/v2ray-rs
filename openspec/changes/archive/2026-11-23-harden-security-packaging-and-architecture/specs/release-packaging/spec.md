# Spec Delta: release-packaging

## ADDED Requirements

### Requirement: CI checks all privileged test features and uninstallers report accurate cleanup
CI SHALL type-check `v2ray-rs-antidpi/privileged-tests` alongside `v2ray-rs-process` and `v2ray-rs-netctl`, `scripts/install.sh --uninstall` SHALL report relocated helper cleanup instead of `setcap -r` on already-deleted prefix binaries, and AppImage scripts and `docs/RELEASE.md` SHALL document `v2ray-rs-antidpi` parity.

#### Scenario: CI checks antidpi privileged tests
- **WHEN** the CI clippy/check workflow runs
- **THEN** `cargo check --workspace --all-targets` SHALL enable `v2ray-rs-antidpi/privileged-tests` alongside the other privileged test features

#### Scenario: Tarball uninstaller prints valid cleanup steps
- **WHEN** `scripts/install.sh --uninstall` removes binaries under `$PREFIX/bin`
- **THEN** its printed follow-up commands SHALL reference `/usr/local/lib/v2ray-rs/` relocated helpers rather than `setcap -r` on deleted `$PREFIX/bin` files
