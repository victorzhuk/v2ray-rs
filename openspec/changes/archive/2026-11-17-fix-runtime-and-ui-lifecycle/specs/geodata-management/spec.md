# Spec Delta

## ADDED Requirements

### Requirement: GeodataIndexManager persists index in cache_dir
`GeodataIndexManager` SHALL read and write `geodata_index.json` under `paths.geodata_index_dir()` (`cache_dir/geodata-index/`) rather than `data_dir`, migrating or ignoring stale `data_dir/geodata_index.json` copies cleanly.

#### Scenario: Index rebuild writes only under cache_dir
- **WHEN** `GeodataIndexManager` saves a rebuilt geodata autocomplete index
- **THEN** `geodata_index.json` SHALL be written under `cache_dir/geodata-index/` and no file SHALL be written under `data_dir`
