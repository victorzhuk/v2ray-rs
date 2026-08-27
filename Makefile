# v2ray-rs Workspace Makefile
# Rust workspace build automation with per-crate targets

# =============================================================================
# Variables
# =============================================================================

CARGO := cargo
CARGO_FLAGS ?=
RUSTFLAGS ?=

# Colors for output
BLUE := \033[34m
GREEN := \033[32m
YELLOW := \033[33m
RED := \033[31m
RESET := \033[0m

# Crate names
CORE := v2ray-rs-core
UI := v2ray-rs-ui
TRAY := v2ray-rs-tray
PROCESS := v2ray-rs-process
SUBSCRIPTION := v2ray-rs-subscription

# =============================================================================
# Default Target
# =============================================================================

.PHONY: default

default: check build test

# =============================================================================
# Build Targets
# =============================================================================

.PHONY: build build-dev \
        build-core build-ui build-tray build-process build-subscription

build:
	@printf "$(BLUE)Building release...$(RESET)\n"
	$(CARGO) build --release $(CARGO_FLAGS)

build-dev:
	@printf "$(BLUE)Building debug...$(RESET)\n"
	$(CARGO) build $(CARGO_FLAGS)

build-core:
	@printf "$(BLUE)Building core crate...$(RESET)\n"
	$(CARGO) build -p $(CORE) --release $(CARGO_FLAGS)

build-ui:
	@printf "$(BLUE)Building UI crate...$(RESET)\n"
	$(CARGO) build -p $(UI) --release $(CARGO_FLAGS)

build-tray:
	@printf "$(BLUE)Building tray crate...$(RESET)\n"
	$(CARGO) build -p $(TRAY) --release $(CARGO_FLAGS)

build-process:
	@printf "$(BLUE)Building process crate...$(RESET)\n"
	$(CARGO) build -p $(PROCESS) --release $(CARGO_FLAGS)

build-subscription:
	@printf "$(BLUE)Building subscription crate...$(RESET)\n"
	$(CARGO) build -p $(SUBSCRIPTION) --release $(CARGO_FLAGS)

# =============================================================================
# Check Targets
# =============================================================================

.PHONY: check check-all clippy fmt fmt-fix

check:
	@printf "$(BLUE)Running cargo check...$(RESET)\n"
	$(CARGO) check $(CARGO_FLAGS)

check-all:
	@printf "$(BLUE)Running cargo check (all targets)...$(RESET)\n"
	$(CARGO) check --all-targets $(CARGO_FLAGS)

clippy:
	@printf "$(BLUE)Running clippy...$(RESET)\n"
	$(CARGO) clippy --workspace --all-targets --all-features $(CARGO_FLAGS) -- -D warnings

fmt:
	@printf "$(BLUE)Checking formatting...$(RESET)\n"
	$(CARGO) fmt -- --check

fmt-fix:
	@printf "$(GREEN)Auto-fixing formatting...$(RESET)\n"
	$(CARGO) fmt

# =============================================================================
# Test Targets
# =============================================================================

.PHONY: test test-core test-ui test-tray test-process test-subscription test-watch

test:
	@printf "$(BLUE)Running all tests...$(RESET)\n"
	$(CARGO) test --workspace --all-targets $(CARGO_FLAGS)

test-core:
	@printf "$(BLUE)Testing core crate...$(RESET)\n"
	$(CARGO) test -p $(CORE) $(CARGO_FLAGS)

test-ui:
	@printf "$(BLUE)Testing UI crate...$(RESET)\n"
	$(CARGO) test -p $(UI) $(CARGO_FLAGS)

test-tray:
	@printf "$(BLUE)Testing tray crate...$(RESET)\n"
	$(CARGO) test -p $(TRAY) $(CARGO_FLAGS)

test-process:
	@printf "$(BLUE)Testing process crate...$(RESET)\n"
	$(CARGO) test -p $(PROCESS) $(CARGO_FLAGS)

test-subscription:
	@printf "$(BLUE)Testing subscription crate...$(RESET)\n"
	$(CARGO) test -p $(SUBSCRIPTION) $(CARGO_FLAGS)

test-watch:
	@printf "$(YELLOW)Running tests in watch mode (requires cargo-watch)...$(RESET)\n"
	$(CARGO) watch -x test

# =============================================================================
# Clean Target
# =============================================================================

.PHONY: clean

clean:
	@printf "$(YELLOW)Cleaning build artifacts...$(RESET)\n"
	$(CARGO) clean
	rm -rf dist

# =============================================================================
# Run Targets
# =============================================================================

.PHONY: run run-dev

run:
	@printf "$(GREEN)Running UI application...$(RESET)\n"
	$(CARGO) run -p $(UI) --release $(CARGO_FLAGS)

run-dev:
	@printf "$(GREEN)Running UI application (dev mode, no tray, separate data)...$(RESET)\n"
	$(CARGO) run -p $(UI) $(CARGO_FLAGS) -- --profile development

# =============================================================================
# Documentation Targets
# =============================================================================

.PHONY: doc doc-open

doc:
	@printf "$(BLUE)Generating documentation...$(RESET)\n"
	$(CARGO) doc --no-deps $(CARGO_FLAGS)

doc-open:
	@printf "$(GREEN)Generating and opening documentation...$(RESET)\n"
	$(CARGO) doc --no-deps --open $(CARGO_FLAGS)

# =============================================================================
# Lint/Quality Targets
# =============================================================================

.PHONY: lint fix

lint: fmt clippy
	@printf "$(GREEN)Lint checks complete.$(RESET)\n"

fix:
	@printf "$(GREEN)Auto-fixing code issues...$(RESET)\n"
	$(CARGO) fix --allow-staged --allow-dirty $(CARGO_FLAGS)
	$(CARGO) fmt
	$(CARGO) clippy --all-features --fix --allow-staged --allow-dirty $(CARGO_FLAGS) -- -D warnings

# =============================================================================
# Release Target
# =============================================================================

.PHONY: release dist

# `target-cpu=native` is deliberately absent: this profile also feeds `dist`,
# and a native build is not portable off the machine that produced it.
release:
	@printf "$(GREEN)Building optimized release...$(RESET)\n"
	$(CARGO) build --release $(CARGO_FLAGS)

# Mirrors the CI tarball. The helpers are built static against musl; the GUI
# links GTK dynamically and inherits this host's glibc, so a tarball produced
# here is only portable to hosts at least as new.
dist: VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
dist:
	@printf "$(BLUE)Building dist artifacts for $(VERSION)...$(RESET)\n"
	$(CARGO) build --release --locked -p v2ray-rs-ui
	$(CARGO) build --release --locked --target x86_64-unknown-linux-musl \
		-p v2ray-rs-netctl -p v2ray-rs-run
	scripts/stage-dist.sh \
		--version "$(VERSION)" \
		--ui target/release/v2ray-rs-ui \
		--netctl target/x86_64-unknown-linux-musl/release/v2ray-rs-netctl \
		--run target/x86_64-unknown-linux-musl/release/v2ray-rs-run \
		--out dist
	tar -C dist -czf dist/v2ray-rs-x86_64-linux.tar.gz \
		"v2ray-rs-$(VERSION)-x86_64-linux"
	cd dist && sha256sum -b v2ray-rs-x86_64-linux.tar.gz \
		> v2ray-rs-x86_64-linux.tar.gz.sha256
	@printf "$(GREEN)dist/v2ray-rs-x86_64-linux.tar.gz$(RESET)\n"

# =============================================================================
# Help Target
# =============================================================================

.PHONY: help

help:
	@printf "$(GREEN)v2ray-rs Workspace Makefile$(RESET)\n"
	@echo ""
	@printf "$(BLUE)Default:$(RESET)\n"
	@echo "  make                Run check, build, and test"
	@echo ""
	@printf "$(BLUE)Build:$(RESET)\n"
	@echo "  make build          Release build"
	@echo "  make build-dev      Debug build"
	@echo "  make build-core     Build core crate"
	@echo "  make build-ui       Build UI crate"
	@echo "  make build-tray     Build tray crate"
	@echo "  make build-process  Build process crate"
	@echo "  make build-subscription Build subscription crate"
	@echo ""
	@printf "$(BLUE)Check:$(RESET)\n"
	@echo "  make check          Run cargo check"
	@echo "  make check-all      Check all targets including tests"
	@echo "  make clippy         Run clippy with all features"
	@echo "  make fmt            Check formatting"
	@echo "  make fmt-fix        Auto-fix formatting"
	@echo ""
	@printf "$(BLUE)Test:$(RESET)\n"
	@echo "  make test           Run all tests"
	@echo "  make test-core      Test core crate"
	@echo "  make test-ui        Test UI crate"
	@echo "  make test-tray      Test tray crate"
	@echo "  make test-process   Test process crate"
	@echo "  make test-subscription Test subscription crate"
	@echo "  make test-watch     Run tests in watch mode"
	@echo ""
	@printf "$(BLUE)Run:$(RESET)\n"
	@echo "  make run            Run the UI application"
	@echo "  make run-dev        Run in dev mode (no tray, separate data)"
	@echo ""
	@printf "$(BLUE)Documentation:$(RESET)\n"
	@echo "  make doc            Generate documentation"
	@echo "  make doc-open       Generate and open docs"
	@echo ""
	@printf "$(BLUE)Quality:$(RESET)\n"
	@echo "  make lint           Run clippy + fmt check"
	@echo "  make fix            Auto-fix clippy and fmt issues"
	@echo "  make clean          Clean build artifacts"
	@echo ""
	@printf "$(BLUE)Release:$(RESET)\n"
	@echo "  make release        Build optimized release"
	@echo "  make dist           Build the release tarball + checksum into dist/"
	@echo ""
	@printf "$(BLUE)Help:$(RESET)\n"
	@echo "  make help           Show this help message"
	@echo ""
	@printf "$(YELLOW)Environment Variables:$(RESET)\n"
	@echo "  CARGO_FLAGS         Additional flags for cargo"
	@echo "  RUSTFLAGS           Additional flags for rustc"
