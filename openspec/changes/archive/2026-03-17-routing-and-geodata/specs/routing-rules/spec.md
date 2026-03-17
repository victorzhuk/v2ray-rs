## ADDED Requirements

### Requirement: GeoData autocomplete in rule input
The rule editor SHALL provide autocomplete for both GeoIP and GeoSite values using the current backend's geodata index.

#### Scenario: GeoIP autocomplete normalizes uppercase
- **WHEN** the user types `r` in a GeoIP field
- **THEN** the system suggests uppercase country codes such as `RU` and stores the selected value in uppercase

#### Scenario: GeoSite autocomplete normalizes lowercase
- **WHEN** the user types `goo` in a GeoSite field
- **THEN** the system suggests lowercase categories such as `google` and stores the selected value in lowercase
