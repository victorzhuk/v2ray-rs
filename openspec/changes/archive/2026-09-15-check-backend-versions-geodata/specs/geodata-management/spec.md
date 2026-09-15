## ADDED Requirements

### Requirement: v2ray and xray connects require referenced geodata
When the selected backend is v2ray or xray and the enabled routing rules, the active imported-profile rules, or the DNS rules used for the connection reference a GeoIP or GeoSite tag, the system SHALL check before starting the connection that `geoip.dat` and `geosite.dat` exist in the profile's geodata directory. When either is missing, Connect SHALL fail without spawning a backend, and the user SHALL see an error stating that the rules need geodata that has not been downloaded, with an action that starts the geodata download for the current backend. sing-box connects SHALL NOT be blocked by missing rule-set files, which fall back to remote rule-sets.

#### Scenario: Missing geodata blocks an xray connect
- **WHEN** the backend is xray, an enabled rule matches GeoSite `google`, and `geosite.dat` does not exist
- **THEN** Connect SHALL fail without spawning a backend and the error SHALL offer a "Download geodata" action

#### Scenario: Download action fetches geodata
- **WHEN** the user invokes "Download geodata" from that error
- **THEN** the system SHALL download the geodata files for the current backend and report success or failure

#### Scenario: Rules without geo references
- **WHEN** the backend is xray, no rule references GeoIP or GeoSite, and no geodata files exist
- **THEN** Connect SHALL proceed

#### Scenario: sing-box is not gated
- **WHEN** the backend is sing-box and a referenced rule-set file is not cached
- **THEN** Connect SHALL proceed
