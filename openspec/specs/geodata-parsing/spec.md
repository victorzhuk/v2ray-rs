## Purpose

Parse downloaded GeoIP/GeoSite databases into a backend-keyed autocomplete index of country codes and categories.

## Requirements

### Requirement: Backend-keyed GeoData autocomplete index
The system SHALL parse downloaded geodata files into a backend-keyed autocomplete index for GeoIP country codes and GeoSite categories.

#### Scenario: v2ray or xray index build
- **WHEN** `.dat` geodata files are refreshed for v2ray or xray
- **THEN** the system extracts GeoIP country codes and GeoSite categories and writes them to a JSON index for that backend

#### Scenario: sing-box index build
- **WHEN** `.db` geodata files are refreshed for sing-box
- **THEN** the system queries the database tables, extracts GeoIP and GeoSite tags, and writes them to a JSON index for that backend
