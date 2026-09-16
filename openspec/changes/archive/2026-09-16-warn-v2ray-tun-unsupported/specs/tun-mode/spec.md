## MODIFIED Requirements

### Requirement: TUN mode availability per backend
The system SHALL offer TUN mode only when the selected backend is sing-box or xray. For the v2ray backend, TUN SHALL be unavailable because v2ray-core has no native TUN inbound. When the user connects with the v2ray backend while TUN is enabled in settings, the connection SHALL proceed without TUN and the system SHALL warn the user, by a toast and by one line in the process log stream, that TUN is not supported by v2ray and only applications using the SOCKS or HTTP proxy at the configured listen address and ports are proxied. For xray, TUN SHALL additionally require Xray-core v26.1.13 or newer (the first release with the `tun` inbound); starting a TUN connection with an older xray SHALL fail before spawn with an error naming the installed and required versions. For xray versions in the range 26.1.13 through 26.6.22 — affected by the upstream TUN crash on quickly-closed connections (Xray-core #6364, fixed in 26.6.27) — the TUN start SHALL proceed but emit an advisory into the process log stream naming the installed version, the crash behavior, and the fixed version. When the installed xray version cannot be read or parsed, the start SHALL proceed and the process log SHALL contain a warning that the version could not be read and the minimum-version check was skipped.

#### Scenario: v2ray backend disables TUN
- **WHEN** the selected backend is v2ray and the user opens the TUN settings page
- **THEN** the enable toggle SHALL be insensitive with an explanatory note, and no tun inbound SHALL be generated even if a stale `enabled` flag is persisted

#### Scenario: Connecting with v2ray while TUN is enabled warns
- **WHEN** TUN is enabled in settings, the backend is v2ray, listen address is `127.0.0.1`, SOCKS port 2080, HTTP port 2081, and the user connects
- **THEN** the connection SHALL start without TUN, a warning toast SHALL state that TUN is not supported by v2ray and only apps using SOCKS `127.0.0.1:2080` or HTTP `127.0.0.1:2081` are proxied, and the process log SHALL contain one line with the same warning

#### Scenario: No warning without the TUN flag
- **WHEN** TUN is disabled in settings and the user connects with v2ray, or TUN is enabled and the backend is sing-box or xray
- **THEN** no v2ray TUN warning SHALL be shown or logged

#### Scenario: sing-box and xray expose TUN
- **WHEN** the selected backend is sing-box or xray
- **THEN** the TUN enable toggle SHALL be available, subject to the capability gate

#### Scenario: Old xray blocks TUN start with a clear error
- **WHEN** TUN is enabled and the detected xray version is older than v26.1.13
- **THEN** the connection preflight SHALL fail with an error stating the installed version and the required minimum, without spawning the backend

#### Scenario: Panic-affected xray warns but starts
- **WHEN** TUN is enabled and the detected xray version is at least 26.1.13 but older than 26.6.27
- **THEN** the connection SHALL start normally and a warning log line SHALL appear in the process logs naming the installed version, the quickly-closed-connection crash, and 26.6.27 as the fixed version

#### Scenario: Unreadable xray version warns
- **WHEN** TUN is enabled with xray and its `version` output cannot be read or parsed
- **THEN** the start SHALL continue and the process log SHALL contain a warning that the version could not be read and the minimum-version check was skipped
