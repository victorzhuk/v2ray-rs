## 1. Generator

- [ ] 1.1 Remove `autoOutboundsInterface` from `build_xray_tun_inbound` in `crates/core/src/config/v2ray.rs`; flip the assertions at `v2ray.rs:1242` and `xray.rs:571` to check the key is absent; `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` green
- [ ] 1.2 Add a test that every outbound of an xray TUN config except `blackhole` and `dns` carries `streamSettings.sockopt.mark == 255` (proxy nodes, `direct`, via-node outbounds); same command green
- [ ] 1.3 Update the xray loop-prevention note in `docs/ARCHITECTURE.md` (policy rules section near the pref 9000 description) to name the fwmark as the only guard; verified by reading the section

## 2. Verification

- [ ] 2.1 `timeout 10m cargo test --workspace -- --test-threads=4` green; `crates/core/tests/xray_check.rs` passes when xray is installed
- [ ] 2.2 Live, with a split-tunnel VPN up (`10.0.0.0/8 via tun0`) and xray TUN connected: `dig <internal-host> @<vpn-dns-ip>` returns the internal A record with no COOKIE warning; `ip route get <node-ip> mark 255` goes via the uplink; proxied browsing works and `backend.log` shows no `tun-in` connection to the node's own address (no loop)
