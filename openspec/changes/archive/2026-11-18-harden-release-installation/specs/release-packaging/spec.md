## ADDED Requirements

### Requirement: Installers require verified downloads
Both release installers SHALL verify the payload SHA-256 before extraction, execution, or prefix modification.

#### Scenario: Verification unavailable or unsuccessful
- **WHEN** the verifier is missing, hashing fails, or the expected digest is unavailable, malformed, or mismatched
- **THEN** installation SHALL exit unsuccessfully and preserve the existing installation, including in quiet mode

#### Scenario: Verified alternative version
- **WHEN** an explicitly selected version has a matching release checksum
- **THEN** both installers SHALL accept its verified payload

### Requirement: Arch helper privileges require restricted access
Arch installation and upgrade SHALL verify root ownership, the v2ray-rs group, and restricted execute permissions before enabling helper privileges.

#### Scenario: Provisioning fails halfway
- **WHEN** either helper fails ownership, permission, privilege, or final-state verification
- **THEN** the hook SHALL report failure and remove any partially applied privileges, reporting any failed cleanup

#### Scenario: Successful provisioning
- **WHEN** provisioning succeeds
- **THEN** netctl SHALL have mode 0750 and CAP_NET_ADMIN and the wrapper SHALL have mode 4750, both owned by root:v2ray-rs

### Requirement: Arch removal preserves system identities
Package removal SHALL preserve the bypass account and access group.

#### Scenario: Existing account and group
- **WHEN** the package is installed and removed on a system with existing service identities
- **THEN** their UID, GID, and existing memberships SHALL remain intact
