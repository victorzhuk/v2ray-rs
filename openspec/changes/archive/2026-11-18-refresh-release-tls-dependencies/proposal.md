## Why

The pre-cut audit of cf0bb6b against RustSec database revision 472a4c9a reports RUSTSEC-2026-0382 in rustls-webpki 0.103.15 and RUSTSEC-2026-0383 in webpki-root-certs 1.0.6. Both advisories concern certificate trust. The release workflow requires a successful advisory audit.

## What Changes

Resolve both advisories in the locked dependency graph and verify the existing HTTPS clients and release checks.

## Capabilities

Existing HTTPS and release behavior stays in force. This dependency maintenance change needs no specification delta.

## Impact

Cargo.lock and any strictly necessary resolver constraints. Review the separate audit warnings before the cut.
