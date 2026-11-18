## 1. Installer integrity

- [x] 1.1 Make both checksum functions fail closed. Preserve working pinned and sidecar verification.
- [x] 1.2 Run both installers against local fixtures for each failure mode. Assert exit status, unchanged prefix, and zero payload execution.
- [x] 1.3 Verify valid pinned and alternate-version installs in fresh prefixes.

## 2. Arch package lifecycle

- [x] 2.1 Enforce ownership and mode prerequisites and verify final privileges in the shared hook.
- [x] 2.2 Revoke partial grants and surface cleanup errors. Preserve accounts and groups on removal.
- [x] 2.3 Exercise install, upgrade, induced permission failure, removal, and reinstall in a disposable environment. Assert helper metadata and account continuity.
- [x] 2.4 Update release documentation with verification prerequisites and recovery behavior.
