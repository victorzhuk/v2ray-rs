# Design: Backend Detection & Selection

## Context

The app wraps system-installed v2ray, xray, and sing-box binaries. It needs to find them, verify they work, and let the user choose which one to use.

## Goals / Non-Goals

**Goals:**
- Detect binaries at well-known paths and via PATH
- Validate binary by running `--version`
- Support user override of binary path
- Provide clear errors when no backend found

**Non-Goals:**
- No binary installation/download
- No protocol-level validation of backends

## Decisions

1. **Search strategy**: Check well-known paths first (`/usr/bin/v2ray`, `/usr/bin/xray`, `/usr/bin/sing-box`, `/usr/local/bin/*`), then fall back to `which` command via PATH. This covers both package-managed and manually installed binaries.

2. **Version detection**: Run `<binary> version` (v2ray/xray) or `<binary> version` (sing-box) and parse stdout. Store version string for display; don't gate on specific versions.

3. **Backend trait**: Define a `Backend` trait abstracting over backend differences (config format, CLI flags, version command). Concrete implementations: `V2rayBackend`, `XrayBackend`, `SingboxBackend`.

## Risks / Trade-offs

- [Binary naming] Some distros may rename binaries (e.g., `v2ray-core`) → Mitigated by allowing user to specify custom path
- [Version command differences] Different backends have different version output formats → Parse loosely, store raw string
