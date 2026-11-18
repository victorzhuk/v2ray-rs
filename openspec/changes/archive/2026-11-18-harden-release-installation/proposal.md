## Why

Both release installers accept downloads when verification is unavailable. The Arch hooks ignore permission errors during privilege setup and delete system identities on removal.

## What Changes

- **BREAKING** Require a valid digest before either installer consumes a download.
- Verify access restrictions before granting privileges during Arch installation and upgrade.
- Preserve system identities across package removal.

## Capabilities

### Modified Capabilities

- `release-packaging` gains installer integrity and Arch helper lifecycle requirements.

## Impact

The tarball and AppImage installers change failure behavior. Both Arch recipes share the affected scriptlet. Existing GUI settings and configuration formats remain compatible.
