## 1. Dialog warning

- [ ] 1.1 In the DNS server dialog, compute effective protocol via `fallback_protocol_for_backend` on protocol selection and dialog open; show/hide an inline warning row naming the effective protocol
- [ ] 1.2 Ensure saving remains allowed with the warning visible

## 2. Saved-row indicator

- [ ] 2.1 In `render_dns_servers`, add a caption-style downgrade indicator on rows whose protocol the active backend downgrades
- [ ] 2.2 Re-render the server list when the active backend type changes so indicators appear/disappear on backend switch

## 3. Spec single-sourcing

- [ ] 3.1 Verify UI call sites use the core compatibility function only (no duplicated matrix); add a convenience helper in `dns.rs` (core) only if the UI needs one

## 4. Tests & verification

- [ ] 4.1 Unit test the helper/predicate for every protocol × backend combination
- [ ] 4.2 `cargo test --workspace` green; `cargo clippy` clean; manual run: v2ray backend + DoQ server shows warning + row indicator, sing-box shows none
