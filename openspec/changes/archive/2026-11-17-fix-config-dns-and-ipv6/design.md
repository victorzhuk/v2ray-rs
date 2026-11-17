# Design

## Context

See `proposal.md` for motivation. Current defects span four closely coupled paths:
1. `crates/core/src/config/singbox.rs` collects `route.rule_set` entries only from `RoutingRule`s (`collect_rule_set_tags`), omitting `DnsRuleMatch::GeoSite` categories and failing `sing-box check` when custom DNS rules reference a GeoSite tag not present in routing. Auto-derived DNS rules in `singbox.rs` also coerce `DomainKeyword` and `DomainFull` into `domain_suffix`.
2. `crates/ui/src/geodata_service.rs` similarly ignores DNS `GeoSite` tags in `singbox_rule_set_tags` and skips any `.srs` file where `manager.has_rule_set(&tag)` is true, leaving cached sing-box rule-sets frozen forever on scheduled and manual updates.
3. `crates/core/src/config/v2ray.rs` mutates the chosen direct DNS server in place inside `attach_exclude_domains`, setting `domains` and `skipFallback: true` on an unrestricted user server so normal queries can no longer use it.
4. `crates/core/src/models/dns.rs`, `crates/subscription/src/parser.rs`, `crates/subscription/src/json_import.rs`, and `crates/subscription/src/ping.rs` mishandle bare vs bracketed IPv6 literals, custom UDP DNS ports, FakeIP CIDR family checks, `xhttp` vs unknown URI transports, Trojan non-443 TLS defaults, and `via_node` lookup in disabled subscriptions.

## Goals / Non-Goals

**Goals:**
- Ensure every generated `sing-box` and `v2ray`/`xray` DNS + routing config passes backend `check`/`-test` across GeoSite DNS rules, `exclude_domains`, IPv6 DNS servers, and `DomainKeyword`/`DomainFull` auto-derivation.
- Ensure scheduled and manual geodata refreshes actually update cached `.srs` files.
- Normalize IPv6 host handling end-to-end (parsers -> models -> ping -> DNS server formatting) on bare IPv6 strings in storage and bracketed authorities on the wire.

**Non-Goals:**
- Consolidating `v2ray.rs`, `singbox.rs`, and `effective_dns.rs` into a single `ResolvedDnsPlan` struct (scheduled in `split-workspace-boundaries` so behavioral fixes land cleanly with minimal diff first).

## Decisions

### 1. Canonical bare IPv6 storage, bracketed wire formatting
- **Choice:** Strip `[` and `]` around valid `Ipv6Addr` literals at parse boundaries (`parser.rs`, `json_import.rs`, `DnsServerConfig` normalization) and reject bracketed strings in `ProxyNode::validate()`. Format authorities with a shared helper (`format_host_port` / `format_url_host`) in `DnsProtocol::server_address()` and `SocketAddr::new(ip, port)` in `ping.rs`.
- **Rationale:** Backend outbound `address`/`server` fields in Xray and sing-box expect bare IPv6 literals (`2001:db8::1`), whereas URLs (`https://[2001:db8::1]/dns-query`) and `host:port` strings (`[2001:db8::1]:5353`) require RFC 3986 brackets. Storing bare IPv6 literals keeps outbound generation uniform and centralizes bracketing at URL/`host:port` formatters.

### 2. Dedicated scoped server entry for `attach_exclude_domains`
- **Choice:** In `v2ray.rs` (and mirrored in `effective_dns.rs`), if the target direct-capable server already carries a `domains` list, append the excluded domains to it; if the server is unrestricted (`domains` absent), insert a dedicated `domains`-scoped `skipFallback: true` clone for `exclude_domains` while leaving the unrestricted server entry untouched.
- **Rationale:** Mutating an unrestricted server into a domain-scoped `skipFallback` server silently breaks general DNS resolution whenever the user has a single direct server.

### 3. Force `.srs` re-download on scheduled and manual refresh
- **Choice:** Split `refresh_singbox_rulesets(force_all: bool)`: pass `force_all = false` on startup missing-geodata check (fast startup when cache exists) and `force_all = true` on scheduled auto-update ticks and manual "Update Now". Expand `singbox_rule_set_tags` (moved to a shared `core` helper) to union routing GeoIP/GeoSite tags with active DNS `GeoSite` tags.
- **Rationale:** Keeps startup fast while fulfilling the `geodata-management` spec contract that scheduled and manual updates refresh rule-sets.

### 4. Parse `xhttp`/`splithttp` in `parser.rs` and fail closed on unknown `type=`
- **Choice:** Match `"tcp" | ""` -> `Tcp`, `"ws"` -> `Ws`, `"grpc"` -> `Grpc`, `"h2" | "http"` -> `H2`, `"xhttp" | "splithttp"` -> `Xhttp(XhttpSettings { path, host, mode })`, and return `Err(ParseError::InvalidFormat)` for any other `type=` / VMess `"net"` value.
- **Rationale:** `ProxyNode` and `json_import.rs` already support `TransportSettings::Xhttp`; adding URI parity lets `xhttp` share links work while preventing unsupported transports (`httpupgrade`, `kcp`, `quic`) from masquerading as broken TCP nodes.

## Risks / Trade-offs

- **[Risk] Rejecting unknown URI `type=` values surfaces skipped-entry warnings on subscriptions with `httpupgrade` or `kcp` nodes** -> `parse_subscription_uris` already isolates per-URI errors and keeps all valid nodes (`subscription-update` spec); surfacing unsupported transports is strictly better than importing non-functional TCP nodes.
- **[Risk] Stricter `DnsConfig` validation (reserved tags, FakeIP family) could flag hand-edited `settings.toml` edge cases** -> Reserved tags (`hosts`, `fakeip`, `sys-dns-bootstrap`, `tun-dns-fallback`) already collide with generated server tags and fail at backend startup; catching them in `DnsConfig::validate` gives a clear diagnostic instead of a backend crash.
