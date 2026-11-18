# Verification

## Advisory gate

`cargo audit --file Cargo.lock` on 2026-11-18, against the current RustSec
database:

- RUSTSEC-2026-0382 (`rustls-webpki` 0.103.15, distrusted intermediates):
  cleared by the 0.103.16 patch on the same minor line.
- RUSTSEC-2026-0383 (`webpki-root-certs` 1.0.6, distrusted emSign roots):
  cleared by 1.0.10.
- No vulnerability advisories remain; the command exits 0.

The 16 remaining allowed warnings are unmaintained, unsound, and yanked-crate
warnings, which CI reports without failing: `chrono` (RUSTSEC-2026-0543),
`paste` (RUSTSEC-2024-0436), `pin-utils` (RUSTSEC-2026-0441), `rustybuzz`
(RUSTSEC-2026-0206), `ttf-parser` (RUSTSEC-2026-0192), `zbus_names`
(RUSTSEC-2026-0371), `zvariant` (RUSTSEC-2026-0370), `zvariant_derive`
(RUSTSEC-2026-0372), `anyhow` (RUSTSEC-2026-0190), `event-listener`
(RUSTSEC-2026-0221), `gettext-rs` (RUSTSEC-2026-0244), `memmap2`
(RUSTSEC-2026-0186), `rand` (RUSTSEC-2026-0097), `rustix`
(RUSTSEC-2026-0486), `tokio` (RUSTSEC-2026-0386), and yanked `spin` 0.9.8.

The resolver diff touches only the two packages above.

## Checks

- `cargo fmt --check --all`
- `cargo check --locked --workspace`
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`
- `V2RAY_RS_REQUIRE_BACKENDS=1 V2RAY_RS_REQUIRE_DISPLAY=1 xvfb-run -a
  dbus-run-session -- make test CARGO_FLAGS="--locked -j 2 --no-fail-fast"
  TEST_TIMEOUT=10m TEST_THREADS=2`: passed.

Certificate verification stays enabled: the tree has no
`danger_accept_invalid_certs`, `danger_accept_invalid_hostnames`, or custom
`ServerCertVerifier` use, and the subscription and probe clients build on
`reqwest` defaults with the ring rustls provider.
