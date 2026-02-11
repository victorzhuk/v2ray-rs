## ADDED Requirements

### Requirement: Download GeoIP and GeoSite databases
The system SHALL download GeoIP and GeoSite databases from upstream sources (v2fly GitHub releases for v2ray/xray, SagerNet releases for sing-box).

#### Scenario: Initial download
- **WHEN** the app launches and no geodata files exist locally
- **THEN** the system SHALL download the appropriate geodata files for the selected backend

#### Scenario: Download failure
- **WHEN** the geodata download fails due to network error
- **THEN** the system SHALL report the error and allow the app to function without geodata (GeoIP/GeoSite rules will be non-functional)

### Requirement: Auto-update geodata
The system SHALL periodically check for and download updated geodata files.

#### Scenario: Weekly update check
- **WHEN** 7 days have elapsed since the last geodata update check
- **THEN** the system SHALL check for new versions and download if available

#### Scenario: Atomic geodata swap
- **WHEN** new geodata is downloaded
- **THEN** the system SHALL replace old files atomically to prevent corruption

### Requirement: Backend-specific geodata format
The system SHALL download the correct geodata format for the selected backend.

#### Scenario: v2ray/xray geodata
- **WHEN** v2ray or xray is the selected backend
- **THEN** the system SHALL use .dat format files (geoip.dat, geosite.dat)

#### Scenario: sing-box geodata
- **WHEN** sing-box is the selected backend
- **THEN** the system SHALL use .db format files (geoip.db, geosite.db)
