## 1. sing-box detour

- [x] 1.1 Emit `detour` only for a proxy detour; a direct detour omits the field
- [x] 1.2 Test: a direct-detoured server carries no `detour` key

## 2. The pin on the sing-box paths

- [x] 2.1 Extract the `hosts` server and its DNS rule into helpers shared by the derived and DNS-feature paths
- [x] 2.2 The derived TUN path emits both
- [x] 2.3 A proxy outbound whose hostname is pinned carries `domain_resolver` naming the `hosts` server
- [x] 2.4 The resolver is named only when a `hosts` server will exist — an unpinned node, an IP-literal node, or a config with no DNS section gets nothing
- [x] 2.5 Tests: pin present on the derived path; pinned outbound names it; unpinned and no-DNS-section cases do not

## 3. Verification

- [x] 3.1 Startup harness that runs the real daemon and fails on `FATAL`, with the TUN inbound stripped and per-case listen ports
- [x] 3.2 Negative control: the harness reproduces `detour to an empty direct outbound makes no sense` on the pre-fix generator and passes after
- [x] 3.3 Workspace suite, clippy, and formatting clean
- [ ] 3.4 Confirm on a live tunnel that a pinned node connects with the DNS feature off, and that excluded domains resolve outside the tunnel
