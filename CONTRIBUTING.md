# Contributing to v2ray-rs

Thank you for your interest in contributing to v2ray-rs! This document provides guidelines and instructions for contributing to the project.

---

## Code of Conduct

- Be respectful and inclusive
- Assume good intentions
- Focus on constructive feedback
- Welcome newcomers and help them learn
- Document decisions and technical discussions

---

## Getting Started

### Prerequisites

- Rust 1.85 or later
- Linux development environment with GTK4/libadwaita dev files
- Git

### First-time Setup

```bash
# Fork and clone your fork
git clone https://github.com/YOUR_USERNAME/v2ray-rs.git
cd v2ray-rs

# Add upstream remote
git remote add upstream https://github.com/victorzhuk/v2ray-rs.git

# Install development dependencies
# Ubuntu/Debian
sudo apt install libgtk-4-dev libadwaita-1-dev

# Build the project
cargo build

# Run tests
cargo test --workspace
```

### Development Environment

```bash
# Install useful tools
cargo install cargo-watch cargo-edit cargo-nextest

# Watch mode for rebuilds
cargo watch -x check -x test -x run

# Faster test runner
cargo nextest run
```

### Local Development Profiles

By default, `cargo run` uses the `development` profile, which stores all data under `~/.config/v2ray-rs-dev/` and `~/.local/share/v2ray-rs-dev/` — completely isolated from any production install.

```bash
# Run with development profile (default for debug builds)
cargo run -p v2ray-rs-ui

# Run with a custom throwaway profile
cargo run -p v2ray-rs-ui -- --profile test

# Reset a profile to clean state
cargo run -p v2ray-rs-ui -- --profile development --reset-instance

# Override just the cache and runtime dirs for CI-like testing
cargo run -p v2ray-rs-ui -- --cache-dir /tmp/v2ray-test/cache --runtime-dir /tmp/v2ray-test/runtime
```

Integration tests use `AppPaths::for_profile_in(AppProfile::Test, tmp.path())` to create isolated directory trees under a temp directory. No feature flags are needed — the constructor is always available.

The legacy `V2RAY_RS_DEV=1` environment variable still maps to the development profile but is deprecated and will be removed in a future release. Use `--profile development` or `V2RAY_RS_PROFILE=development` instead.

---

## Development Workflow

### 1. Check for Existing Issues

Search [GitHub Issues](https://github.com/victorzhuk/v2ray-rs/issues) to avoid duplicate work.

### 2. Create a Branch

```bash
git checkout master
git pull upstream master
git checkout -b feature/description-or-issue-number
```

Branch naming conventions:
- `feature/` - New features
- `fix/` - Bug fixes
- `refactor/` - Code refactoring
- `docs/` - Documentation updates
- `test/` - Test additions or improvements

### 3. Make Changes

Follow the coding standards below and run tests frequently.

### 4. Commit Changes

```bash
# Stage changes
git add .

# Commit with conventional commits
git commit -m "feat: add support for VLESS protocol"
```

Commit message format:
- `feat:` - New feature
- `fix:` - Bug fix
- `refactor:` - Code refactoring
- `docs:` - Documentation changes
- `test:` - Test changes
- `chore:` - Maintenance tasks

### 5. Test Your Changes

```bash
# Run all tests
cargo test --workspace

# Run with race detector
cargo test --workspace --release --all-features

# Run linter
cargo clippy --workspace -- -D warnings

# Format code
cargo fmt --all
```

### 6. Push and Create Pull Request

```bash
git push origin feature/description
```

Create a PR from your fork's branch to `upstream/master`.

### 7. Address Feedback

Respond to reviewer comments, make requested changes, and push updates.

---

## Coding Standards

### Rust Conventions

Follow standard Rust conventions:

#### Naming

```rust
// Variables: natural, concise
let cfg = Config::new();
let repo = Repository::new(pool);
let ctx = &Context::new();

// Structs: Private by default
pub struct ProxyNode {  // Public domain type
    pub id: Uuid,
    pub protocol: String,
}

struct ProcessManager {  // Private implementation type
    state: Arc<RwLock<State>>,
}

// Constructors: New() for public, new_*() for internal
impl ProxyNode {
    pub fn new(config: ProxyConfig) -> Result<Self> { ... }
}

fn new_internal_state() -> State { ... }
```

#### Error Handling

```rust
// Use thiserror for error types
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("invalid proxy URL: {0}")]
    InvalidUrl(String),

    #[error("backend not found: {0}")]
    BackendNotFound(String),
}

// Wrap errors with context
fn load_config(path: PathBuf) -> Result<Config, ConfigError> {
    let content = fs::read_to_string(&path)
        .map_err(|e| ConfigError::ReadFailed(path.clone(), e))?;
    // ...
}
```

#### Documentation

```rust
/// Loads subscription from a URL or local file.
///
/// # Arguments
///
/// * `source` - Subscription source (URL or file path)
///
/// # Returns
///
/// Returns a `Subscription` on success, or an error if:
/// - URL is invalid
/// - HTTP request fails
/// - Parsing fails
///
/// # Examples
///
/// ```
/// use v2ray_rs_subscription::SubscriptionFetcher;
///
/// let fetcher = SubscriptionFetcher::new();
/// let sub = fetcher.fetch("https://example.com/sub").await?;
/// # Ok::<(), v2ray_rs_subscription::Error>(())
/// ```
pub async fn fetch(&self, source: &str) -> Result<Subscription, Error> {
    // ...
}
```

### Clean Architecture

```
┌─────────────────────────────────────┐
│   Presentation (Relm4 UI, CLI)       │  ← Depends on Application
├─────────────────────────────────────┤
│   Application (Use Cases, Managers) │  ← Depends on Domain
├─────────────────────────────────────┤
│   Domain (Models, Entities)         │  ← Zero dependencies
├─────────────────────────────────────┤
│   Infrastructure (IO, HTTP, DB)     │  ← Depends on Domain
└─────────────────────────────────────┘
```

Rules:
- Domain layer has **zero external dependencies**
- Dependencies flow **inward only**
- Domain types live in `v2ray-rs-core`
- Use concrete types, not interfaces (unless truly needed)
- Prefer composition over inheritance

### Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_subscription_parsing() {
        let raw = "vless://uuid@example.com:443?encryption=none#node1";
        let node = parse_proxy_uri(raw).unwrap();

        assert_eq!(node.protocol, ProxyProtocol::Vless);
        assert_eq!(node.address, "example.com");
    }

    #[test]
    fn test_invalid_subscription_url() {
        let result = parse_proxy_uri("invalid://url");
        assert!(result.is_err());
    }

    // Table-driven tests
    #[test]
    fn test_geoip_validation() {
        let tests = vec![
            ("CN", true),
            ("US", true),
            ("XX", false),  // Invalid
            ("", false),
        ];

        for (code, valid) in tests {
            assert_eq!(validate_country_code(code).is_ok(), valid);
        }
    }
}
```

### Code Style

- Use `cargo fmt` for formatting
- Run `cargo clippy` before committing
- Keep functions focused and small (< 50 lines when possible)
- Avoid magic numbers; use named constants
- Prefer `Result<T, E>` over panicking
- Use `?` operator for error propagation
- No `unwrap()` in production code (use `context` in tests)

---

## Project Structure

```
crates/
├── core/              # Domain models + infrastructure
│   ├── src/
│   │   ├── models/    # Domain entities
│   │   ├── config/    # Backend config generation
│   │   ├── persistence.rs
│   │   ├── backend.rs
│   │   ├── geodata.rs
│   │   └── routing_manager.rs
├── subscription/      # Subscription fetching/parsing
├── process/           # Process lifecycle management
├── tray/              # System tray (StatusNotifierItem)
└── ui/                # GTK4/Relm4 GUI
```

### Adding New Features

1. **Domain Changes**: Add to `crates/core/src/models/`
2. **New Protocol Support**: Extend `ProxyNode` enum in `crates/core/src/models/proxy.rs`
3. **Backend Config**: Add to `crates/core/src/config/`
4. **New Tests**: Add test modules alongside implementation

---

## OpenSpec Workflow

We use OpenSpec for spec-driven development.

### Starting a New Feature

```bash
# Create a new change
openspec new change feature-name

# Show status
openspec list
openspec show feature-name
```

### Fast-Forward (Recommended)

```bash
# Create all artifacts at once
openspec ff feature-name

# Implement tasks from tasks.md
# ...

# Validate and archive
openspec validate feature-name
openspec archive feature-name
```

### Incremental

```bash
# Create change
openspec new change feature-name

# Create artifacts one by one
openspec continue feature-name  # Creates next artifact
openspec continue feature-name  # Repeat...

# Archive when done
openspec archive feature-name
```

### Artifacts Structure

```
openspec/changes/feature-name/
├── proposal.md      # Requirements, user stories
├── design.md        # Architecture, data flow
├── tasks.md         # Implementation checklist
├── specs/           # Delta specs per component
│   ├── core.md
│   ├── subscription.md
│   └── process.md
└── README.md        # Change summary
```

### Agent Delegation

See `.opencode/skills/openspec-apply-change/SKILL.md` for agent workflow guidance.

---

## Reporting Issues

### Bug Reports

Include in your issue:

1. **Description**: Clear explanation of the bug
2. **Reproduction Steps**: How to reproduce
3. **Expected Behavior**: What should happen
4. **Actual Behavior**: What actually happens
5. **Environment**:
   - OS and version
   - v2ray-rs version
   - Installed backends (v2ray/xray/sing-box versions)
6. **Logs**: Relevant log output
7. **Screenshots**: If UI-related

Example template:

```markdown
**Describe the bug**
Subscription import fails with timeout error.

**To Reproduce**
1. Click File → Import from URL
2. Enter https://example.com/subscription
3. Click Import
4. Error appears

**Expected**
Subscription imported successfully

**Actual**
Error: "HTTP request failed: timeout after 60s"

**Environment**
- OS: Ubuntu 24.04
- Version: 0.1.0
- Backend: xray 1.8.6
```

### Feature Requests

Include in your feature request:

1. **Summary**: One-line description
2. **Use Case**: Why this feature is needed
3. **Proposed Solution**: How you envision it working
4. **Alternatives**: Other approaches considered
5. **Priority**: Low/Medium/High

Example template:

```markdown
**Feature Summary**
Add support for SOCKS5 authentication

**Use Case**
Many corporate SOCKS5 proxies require username/password authentication.

**Proposed Solution**
Add `username` and `password` fields to `ProxyNode` for SOCKS5 protocol.

**Alternatives**
1. Use HTTP proxy instead
2. Use separate authentication config

**Priority**
Medium
```

---

## Feature Requests

For large features, we recommend:

1. **Discuss First**: Open an issue or discussion before coding
2. **Use OpenSpec**: Create a change with proposal/design
3. **Break Down**: Use `openspec ff` to create tasks
4. **Iterate**: Implement incrementally

---

## Getting Help

### Communication Channels

- **GitHub Issues**: Bug reports, feature requests
- **GitHub Discussions**: Questions, ideas, community topics
- **Discord/Matrix**: *(coming soon)*

### Resources

- [README.md](README.md) - Project overview
- [CLAUDE.md](CLAUDE.md) - Development guidelines
- [openspec/](openspec/) - Feature specifications

### Common Issues

**Build fails with "cannot find -lgtk-4"**
```bash
sudo apt install libgtk-4-dev libadwaita-1-dev
```

**Tests fail on macOS**
v2ray-rs is Linux-only (GTK4/libadwaita dependency). For development, use WSL2 or a Linux VM.

---

## License

By contributing, you agree that your contributions will be licensed under the Apache License 2.0.

---

## Recognition

Contributors are recognized in:
- GitHub release notes
- Project documentation

Thank you for contributing to v2ray-rs!
