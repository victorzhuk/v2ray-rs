# Tasks: Backend Detection & Selection

## 1. Backend Abstraction

- [x] 1.1 Define Backend trait with methods: name(), binary_path(), version_command(), config_flag()
- [x] 1.2 Implement V2rayBackend, XrayBackend, SingboxBackend structs
- [x] 1.3 Write unit tests for each backend implementation

## 2. Binary Detection

- [x] 2.1 Implement well-known path scanning (/usr/bin/, /usr/local/bin/) for all three backends
- [x] 2.2 Implement PATH-based fallback using `which` command
- [x] 2.3 Implement version detection by running binary with version argument and parsing stdout
- [x] 2.4 Handle binary-exists-but-fails-to-run case (permission denied, corrupt binary)
- [x] 2.5 Write tests for detection logic (with mock binaries)

## 3. Backend Selection

- [x] 3.1 Implement auto-selection when only one backend is found
- [x] 3.2 Implement multi-backend selection (return list of available backends)
- [x] 3.3 Implement custom binary path override with validation (exists + executable)
- [x] 3.4 Integrate with persistence layer to save/load backend selection

## 4. Error Handling

- [x] 4.1 Define BackendError types for: not found, not executable, version parse failure
- [x] 4.2 Create user-friendly error messages with installation guidance per backend
- [x] 4.3 Write tests for error scenarios
