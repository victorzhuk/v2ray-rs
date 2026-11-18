# Spec Delta

## ADDED Requirements

### Requirement: Application localization applies message catalogs without altering numeric locale
At startup, the UI SHALL initialize `gettext` for `LC_MESSAGES` using the persisted `Language` setting without overriding `LC_NUMERIC` or `LC_ALL`, and SHALL look up primary window, status bar, Preferences, and dialog strings through the translation catalog. Changing `Language` in Preferences SHALL persist the choice and inform the user that restarting the application completes full UI relabeling.

#### Scenario: Russian language selection translates UI without comma decimal separators
- **WHEN** `settings.language` is `Russian` (`ru_RU`) at application startup
- **THEN** primary window and Preferences labels SHALL resolve from the `ru_RU` message catalog while floating-point formatting under `LC_NUMERIC` continues using `.` decimal separators

#### Scenario: Language change surfaces restart notice
- **WHEN** the user changes the `Language` dropdown in System Preferences while the app is running
- **THEN** the new `Language` SHALL be persisted and the UI SHALL display a non-blocking notice stating that an application restart is required to relabel open views
