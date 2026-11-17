# Spec Delta

## ADDED Requirements

### Requirement: Rule-set tags are validated before path or URL construction
The system SHALL validate every GeoIP and GeoSite rule-set tag against `[a-z0-9][a-z0-9_-]*` after canonical lowercasing before constructing local `.srs` cache paths or remote download URLs, rejecting empty tags, path separators, parent-directory segments, and non-ASCII characters.

#### Scenario: Traversal tag rejected before file lookup or download
- **WHEN** a rule-set lookup or download receives a tag containing `../`, `/`, `\`, or whitespace
- **THEN** the operation SHALL return a validation error without touching the filesystem or issuing an HTTP request

#### Scenario: Standard GeoIP and GeoSite tags pass validation
- **WHEN** a rule-set lookup or download receives `geoip-ru` or `geosite-category-ads-all`
- **THEN** validation SHALL succeed and resolve under `cache_dir/geodata/rule-sets/`
