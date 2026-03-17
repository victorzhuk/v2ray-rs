# Proposal: Routing & GeoData Enhancements

## Why
Routing rule order determines precedence, so users need drag-and-drop reordering instead of only move buttons. Users also need indexed GeoIP and GeoSite data for autocomplete and validation, but this change should improve indexing and visibility without introducing editable geodata source management.

## What Changes
- **Page-local drag-and-drop**: Implement DnD for routing rules inside the existing Preferences page and reuse the current save/regenerate path.
- **GeoData indexing**: Parse downloaded `.dat` and `.db` geodata into a backend-keyed autocomplete index.
- **Autocomplete suggestions**: Offer GeoIP and GeoSite suggestions with the same normalization rules used by validation.
- **GeoData status UI**: Show last successful refresh time, indexed tag counts, and a manual "Update Now" action. Editable geodata sources remain out of scope.

## Capabilities

### New Capabilities
- `ui-drag-and-drop`
- `geodata-parsing`

### Modified Capabilities
- `routing-rules`
- `geodata-management`
