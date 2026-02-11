# v2ray-rs Workspace Makefile
# Rust workspace build automation with per-crate targets

# =============================================================================
# Variables
# =============================================================================

CARGO := cargo
CARGO_FLAGS ?=
RUSTFLAGS ?=
CARGO_TARGET_DIR ?=

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
	@echo "$(BLUE)Building release...$(RESET)"
	$(CARGO) build --release $(CARGO_FLAGS)

build-dev:
	@echo "$(BLUE)Building debug...$(RESET)"
	$(CARGO) build $(CARGO_FLAGS)

build-core:
	@echo "$(BLUE)Building core crate...$(RESET)"
	$(CARGO) build -p $(CORE) --release $(CARGO_FLAGS)

build-ui:
	@echo "$(BLUE)Building UI crate...$(RESET)"
	$(CARGO) build -p $(UI) --release $(CARGO_FLAGS)

build-tray:
	@echo "$(BLUE)Building tray crate...$(RESET)"
	$(CARGO) build -p $(TRAY) --release $(CARGO_FLAGS)

build-process:
	@echo "$(BLUE)Building process crate...$(RESET)"
	$(CARGO) build -p $(PROCESS) --release $(CARGO_FLAGS)

build-subscription:
	@echo "$(BLUE)Building subscription crate...$(RESET)"
	$(CARGO) build -p $(SUBSCRIPTION) --release $(CARGO_FLAGS)

# =============================================================================
# Check Targets
# =============================================================================

.PHONY: check check-all clippy fmt fmt-fix

check:
	@echo "$(BLUE)Running cargo check...$(RESET)"
	$(CARGO) check $(CARGO_FLAGS)

check-all:
	@echo "$(BLUE)Running cargo check (all targets)...$(RESET)"
	$(CARGO) check --all-targets $(CARGO_FLAGS)

clippy:
	@echo "$(BLUE)Running clippy...$(RESET)"
	$(CARGO) clippy --all-features -- -D warnings $(CARGO_FLAGS)

fmt:
	@echo "$(BLUE)Checking formatting...$(RESET)"
	$(CARGO) fmt -- --check

fmt-fix:
	@echo "$(GREEN)Auto-fixing formatting...$(RESET)"
	$(CARGO) fmt

# =============================================================================
# Test Targets
# =============================================================================

.PHONY: test test-core test-ui test-tray test-process test-subscription test-watch

test:
	@echo "$(BLUE)Running all tests...$(RESET)"
	$(CARGO) test --workspace $(CARGO_FLAGS)

test-core:
	@echo "$(BLUE)Testing core crate...$(RESET)"
	$(CARGO) test -p $(CORE) $(CARGO_FLAGS)

test-ui:
	@echo "$(BLUE)Testing UI crate...$(RESET)"
	$(CARGO) test -p $(UI) $(CARGO_FLAGS)

test-tray:
	@echo "$(BLUE)Testing tray crate...$(RESET)"
	$(CARGO) test -p $(TRAY) $(CARGO_FLAGS)

test-process:
	@echo "$(BLUE)Testing process crate...$(RESET)"
	$(CARGO) test -p $(PROCESS) $(CARGO_FLAGS)

test-subscription:
	@echo "$(BLUE)Testing subscription crate...$(RESET)"
	$(CARGO) test -p $(SUBSCRIPTION) $(CARGO_FLAGS)

test-watch:
	@echo "$(YELLOW)Running tests in watch mode (requires cargo-watch)...$(RESET)"
	$(CARGO) watch -x test

# =============================================================================
# Clean Target
# =============================================================================

.PHONY: clean

clean:
	@echo "$(YELLOW)Cleaning build artifacts...$(RESET)"
	$(CARGO) clean

# =============================================================================
# Run Targets
# =============================================================================

.PHONY: run run-dev

run:
	@echo "$(GREEN)Running UI application...$(RESET)"
	$(CARGO) run -p $(UI) --release $(CARGO_FLAGS)

run-dev:
	@echo "$(GREEN)Running UI application (debug)...$(RESET)"
	$(CARGO) run -p $(UI) $(CARGO_FLAGS)

# =============================================================================
# Documentation Targets
# =============================================================================

.PHONY: doc doc-open

doc:
	@echo "$(BLUE)Generating documentation...$(RESET)"
	$(CARGO) doc --no-deps $(CARGO_FLAGS)

doc-open:
	@echo "$(GREEN)Generating and opening documentation...$(RESET)"
	$(CARGO) doc --no-deps --open $(CARGO_FLAGS)

# =============================================================================
# Lint/Quality Targets
# =============================================================================

.PHONY: lint fix

lint: fmt clippy
	@echo "$(GREEN)Lint checks complete.$(RESET)"

fix:
	@echo "$(GREEN)Auto-fixing code issues...$(RESET)"
	$(CARGO) fix --allow-staged --allow-dirty $(CARGO_FLAGS)
	$(CARGO) fmt
	$(CARGO) clippy --all-features --fix --allow-staged --allow-dirty -- -D warnings $(CARGO_FLAGS)

# =============================================================================
# Release Target
# =============================================================================

.PHONY: release

release:
	@echo "$(GREEN)Building optimized release...$(RESET)"
	RUSTFLAGS="-C target-cpu=native $(RUSTFLAGS)" $(CARGO) build --release $(CARGO_FLAGS)

# =============================================================================
# Help Target
# =============================================================================

.PHONY: help

help:
	@echo "$(GREEN)v2ray-rs Workspace Makefile$(RESET)"
	@echo ""
	@echo "$(BLUE)Default:$(RESET)"
	@echo "  make                Run check, build, and test"
	@echo ""
	@echo "$(BLUE)Build:$(RESET)"
	@echo "  make build          Release build"
	@echo "  make build-dev      Debug build"
	@echo "  make build-core     Build core crate"
	@echo "  make build-ui       Build UI crate"
	@echo "  make build-tray     Build tray crate"
	@echo "  make build-process  Build process crate"
	@echo "  make build-subscription Build subscription crate"
	@echo ""
	@echo "$(BLUE)Check:$(RESET)"
	@echo "  make check          Run cargo check"
	@echo "  make check-all      Check all targets including tests"
	@echo "  make clippy         Run clippy with all features"
	@echo "  make fmt            Check formatting"
	@echo "  make fmt-fix        Auto-fix formatting"
	@echo ""
	@echo "$(BLUE)Test:$(RESET)"
	@echo "  make test           Run all tests"
	@echo "  make test-core      Test core crate"
	@echo "  make test-ui        Test UI crate"
	@echo "  make test-tray      Test tray crate"
	@echo "  make test-process   Test process crate"
	@echo "  make test-subscription Test subscription crate"
	@echo "  make test-watch     Run tests in watch mode"
	@echo ""
	@echo "$(BLUE)Run:$(RESET)"
	@echo "  make run            Run the UI application"
	@echo "  make run-dev        Run in debug mode"
	@echo ""
	@echo "$(BLUE)Documentation:$(RESET)"
	@echo "  make doc            Generate documentation"
	@echo "  make doc-open       Generate and open docs"
	@echo ""
	@echo "$(BLUE)Quality:$(RESET)"
	@echo "  make lint           Run clippy + fmt check"
	@echo "  make fix            Auto-fix clippy and fmt issues"
	@echo "  make clean          Clean build artifacts"
	@echo ""
	@echo "$(BLUE)Release:$(RESET)"
	@echo "  make release        Build optimized release"
	@echo ""
	@echo "$(BLUE)Help:$(RESET)"
	@echo "  make help           Show this help message"
	@echo ""
	@echo "$(YELLOW)Environment Variables:$(RESET)"
	@echo "  CARGO_FLAGS         Additional flags for cargo"
	@echo "  RUSTFLAGS           Additional flags for rustc"
	@echo "  CARGO_TARGET_DIR    Override target directory"
