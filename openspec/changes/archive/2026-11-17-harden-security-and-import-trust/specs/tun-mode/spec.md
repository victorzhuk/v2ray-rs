# Spec Delta

## ADDED Requirements

### Requirement: Strict IPv6 TUN captures port-53 DNS without an IPv6 tunnel address
When `xray-up` runs with `--strict` and `--capture-dns` on a host with IPv6 available, the helper SHALL install the IPv6 port-53 DNS capture rules (pref 8999) into table 2023 even when no IPv6 tunnel address is configured, so unmarked queries to on-link IPv6 resolvers hit the strict unreachable route instead of leaking on the local link.

#### Scenario: On-link IPv6 DNS blocked under IPv4-only strict xray TUN
- **WHEN** `xray-up` runs with `--strict` and `--capture-dns`, no IPv6 tunnel address is supplied, and the host has an on-link IPv6 resolver (`fe80::1` or a `/64` LAN address)
- **THEN** unmarked IPv6 UDP and TCP packets to port 53 SHALL match the pref 8999 rule into table 2023 and hit the strict `unreachable` route rather than leaving on the LAN link

#### Scenario: Host with IPv6 disabled skips IPv6 DNS capture
- **WHEN** `xray-up` runs with `--strict` and `--capture-dns` on a host with IPv6 disabled
- **THEN** no IPv6 port-53 capture rule SHALL be installed and IPv4 setup SHALL succeed

### Requirement: Legacy helper override requires root
The route helper's `--force-legacy` flag SHALL require effective user ID 0. Invocations with `--force-legacy` by non-root accounts holding only file capabilities via the `v2ray-rs` group SHALL fail before opening netlink or mutating kernel state.

#### Scenario: Unprivileged group member rejected on force-legacy
- **WHEN** a non-root user in the `v2ray-rs` group invokes `v2ray-rs-netctl --force-legacy recover --xray` directly
- **THEN** the helper SHALL exit with a permission error and SHALL make no netlink or journal changes

#### Scenario: Root operator recovery allowed on force-legacy
- **WHEN** an operator invokes `v2ray-rs-netctl --force-legacy recover --xray` with effective UID 0
- **THEN** the helper SHALL execute the legacy cleanup pass
