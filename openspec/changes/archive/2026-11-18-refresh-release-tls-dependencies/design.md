## Decisions

Prefer compatible patch resolutions. The audited database lists rustls-webpki 0.103.16 on the current minor line and webpki-root-certs 1.0.10 as patched versions. Check the current database before selecting versions. Inspect reverse dependencies and preserve the existing toolchain floor and TLS provider.

## Validation and recovery

Run the same advisory check as CI and the locked workspace acceptance suite. Review the resolver diff for unrelated upgrades. If a compatible patch cannot resolve, record the constraining parent dependency before broadening the upgrade.
