## 1. GeoData indexing

- [x] 1.1 Add `prost` parsing for v2ray/xray `.dat` files and `rusqlite` queries for sing-box `.db` files
- [x] 1.2 Persist a backend-keyed JSON autocomplete index with tag lists, last successful refresh time, and tag counts
- [x] 1.3 Preserve the previous index and metadata if download or parsing fails

## 2. Autocomplete UI

- [x] 2.1 Add GeoIP and GeoSite autocomplete to the routing-rule dialog from the current backend's index
- [x] 2.2 Normalize GeoIP values to uppercase and GeoSite values to lowercase before saving
- [x] 2.3 Verify autocomplete suggestions match the validation rules

## 3. Drag-and-drop reordering

- [x] 3.1 Implement GTK drag-and-drop on the routing-rule rows in `crates/ui/src/preferences.rs`
- [x] 3.2 Reuse the existing save-and-regenerate path after a successful drop
- [x] 3.3 Ensure the list re-renders in the new order immediately

## 4. GeoData settings view

- [x] 4.1 Add a "GeoData" `adw::PreferencesGroup` to the settings page
- [x] 4.2 Display last successful refresh time and indexed tag counts for GeoIP and GeoSite
- [x] 4.3 Add an "Update Now" button that triggers download and reindex for the current backend
