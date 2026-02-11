## Context

Users import proxy subscriptions from URLs (base64-encoded lists of proxy URIs) or local files. The app must parse various URI formats (vless://, vmess://, ss://, trojan://) and extract node details.

## Goals / Non-Goals

**Goals:**
- Parse base64-encoded subscription responses into individual proxy URIs
- Parse VLESS, VMess (including base64 JSON format), Shadowsocks, and Trojan URI schemes
- Support file-based import
- Background auto-update with configurable interval
- Graceful error handling for network/parse failures

**Non-Goals:**
- No SIP008 (Shadowsocks JSON) format initially
- No subscription creation/hosting

## Decisions

1. **URI parsing**: Implement dedicated parsers for each URI scheme. VMess uses base64-encoded JSON (`vmess://base64json`). VLESS/Trojan use standard URI format. Shadowsocks uses SIP002 format (`ss://base64(method:password)@host:port`).

2. **HTTP client**: Use `reqwest` with async runtime for subscription fetching. Set reasonable timeouts (30s connect, 60s total). Follow redirects. Support custom User-Agent.

3. **Auto-update strategy**: Use a tokio interval timer. On failure, retry with exponential backoff (max 3 retries). Notify UI of update status changes.

4. **Storage**: Subscriptions stored as JSON in data directory. Each subscription gets its own file: `subscriptions/<uuid>.json`.

## Risks / Trade-offs

- [URI format variations] Real-world subscription providers use inconsistent URI formats → Mitigated by lenient parsing with fallback heuristics
- [Large subscriptions] Some subscriptions contain hundreds of nodes → Handle lazily, don't load all into memory at once in UI
