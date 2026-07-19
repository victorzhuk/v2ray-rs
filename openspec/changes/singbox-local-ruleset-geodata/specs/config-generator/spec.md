## ADDED Requirements

### Requirement: sing-box rule-sets prefer local cached files
When generating a sing-box config, each referenced GeoIP/GeoSite rule-set SHALL be emitted as `type: "local"` with `format: "binary"` and the absolute path of the cached `.srs` file when that file exists in the geodata cache, and as `type: "remote"` otherwise.

#### Scenario: Cached tag becomes a local rule-set
- **WHEN** `geosite-yandex.srs` exists under the geodata rule-set cache and a routing rule references GeoSite "yandex"
- **THEN** the emitted rule-set entry SHALL be `{"type": "local", "format": "binary", "path": "<cache>/geosite-yandex.srs"}` for tag `geosite-yandex`

#### Scenario: Uncached tag falls back to remote
- **WHEN** no cached `.srs` exists for a referenced tag
- **THEN** the emitted rule-set entry for that tag SHALL be `type: "remote"` with the upstream URL

#### Scenario: Mixed local and remote sets coexist
- **WHEN** some referenced tags are cached and others are not
- **THEN** the config SHALL contain local entries for the cached tags and remote entries for the rest, and `experimental.cache_file` SHALL be enabled while any remote entry is present
