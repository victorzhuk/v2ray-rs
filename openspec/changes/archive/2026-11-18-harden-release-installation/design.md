## Context

Both verify_checksum functions return success after warning about a missing verifier or digest. Both Arch recipes stage helpers with mode 0755. The shared post_install suppresses chmod errors before setcap. Its pre_remove deletes the named account and group even when post_install found them already present.

## Goals

Keep unverified bytes out of installation prefixes. Keep privileged helper execution restricted to the intended group. Preserve local account ownership across uninstall.

## Decisions

Keep the existing digest selection order. Abort before extraction, execution, or prefix writes if verification fails. Apply the same policy to version overrides and quiet installs.

Start from unprivileged helpers. Verify ownership and restrictive access before enabling either privilege. Check final state and revoke partial grants on failure. Surface recovery errors explicitly. Retain system accounts and groups on removal to preserve identity across reinstalls.

## Risks and recovery

Offline mirrors need a matching digest. A provisioning failure can leave TUN unavailable. Report the affected helper and keep it disabled until an operator repairs its permissions. Exercise failure cases in a disposable Arch environment.
