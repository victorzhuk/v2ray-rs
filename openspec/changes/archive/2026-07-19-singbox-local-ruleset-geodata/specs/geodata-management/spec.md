## MODIFIED Requirements

### Requirement: Backend-specific geodata format
The system SHALL download the correct geodata format for the selected backend. For sing-box that format is per-tag binary rule-set files (`.srs`); the legacy `geoip.db`/`geosite.db` files are unsupported by sing-box 1.12+ and SHALL NOT be downloaded or kept.

#### Scenario: v2ray/xray geodata
- **WHEN** v2ray or xray is the selected backend
- **THEN** the system SHALL use .dat format files (geoip.dat, geosite.dat)

#### Scenario: sing-box geodata
- **WHEN** sing-box is the selected backend
- **THEN** the system SHALL download per-tag `.srs` rule-set files for the GeoIP/GeoSite tags referenced by the current routing rules into `cache_dir/geodata/rule-sets/`

#### Scenario: Stale .db files removed
- **WHEN** a geodata refresh runs for sing-box and legacy `geoip.db`/`geosite.db` files exist in the cache
- **THEN** the system SHALL delete them

### Requirement: Download GeoIP and GeoSite databases
The system SHALL download GeoIP and GeoSite data from upstream sources (v2fly GitHub releases for v2ray/xray; the `rule-set` branches of SagerNet/sing-geoip and SagerNet/sing-geosite for sing-box).

#### Scenario: Initial download
- **WHEN** the app launches and no geodata files exist locally
- **THEN** the system SHALL download the appropriate geodata files for the selected backend

#### Scenario: Download failure
- **WHEN** the geodata download fails due to network error
- **THEN** the system SHALL report the error and allow the app to function without geodata (for sing-box, affected tags fall back to remote rule-sets in the generated config)

#### Scenario: New tag referenced by routing rules
- **WHEN** a routing rule referencing a GeoIP/GeoSite tag with no cached `.srs` is added and sing-box is the selected backend
- **THEN** the next refresh pass (startup, scheduled, or manual) SHALL fetch the missing tag's `.srs` file
