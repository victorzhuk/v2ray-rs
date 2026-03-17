# Design: Routing & GeoData Enhancements

## Context
Routing rule order is critical for correct proxy behavior. The current UI requires clicking "Move Up/Down" buttons, which is tedious. Geodata downloads also remain opaque files, so users cannot see or reuse the valid GeoIP and GeoSite tags that already exist on disk.

## Architecture

### 1. Drag-and-drop reordering
- Keep routing-rule reordering inside `crates/ui/src/preferences.rs`.
- A completed drop mutates the local `RoutingRuleSet`, persists it immediately, and triggers the same config-regeneration path as the existing move buttons.

### 2. Backend-keyed GeoData index
- For v2ray/xray `.dat`, use `prost` to extract GeoIP country codes and GeoSite categories.
- For sing-box `.db`, use `rusqlite` to query the stored tag tables.
- Persist a backend-keyed JSON index next to the downloaded files, containing per-dataset tag lists, last successful refresh time, and tag counts.
- If download or indexing fails, keep the previous index and metadata intact.

### 3. Autocomplete and normalization
- The routing-rule dialog loads the current backend's geodata index.
- GeoIP suggestions are uppercase; GeoSite suggestions are lowercase.
- The UI normalizes the selected value before saving so autocomplete and validation agree.

### 4. GeoData management UI
- Add a new `adw::PreferencesGroup` titled "GeoData" in the settings.
- Display the last successful refresh time plus indexed tag counts for GeoIP and GeoSite.
- Provide an "Update Now" button that triggers download plus reindex for the current backend.
- Editable source URLs remain out of scope for this change.
