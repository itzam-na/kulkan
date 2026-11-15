.DEFAULT_GOAL := help

CARGO ?= cargo
MAKEPKG ?= makepkg
SUDO ?= sudo
PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share
DESTDIR ?=
CARGO_TARGET_DIR ?= target
CARGO_FETCH_FLAGS ?= --locked
CARGO_BUILD_FLAGS ?= --locked
CARGO_TEST_FLAGS ?= --locked
CARGO_CLIPPY_FLAGS ?= --locked
TARGET ?=

export CARGO_TARGET_DIR

.PHONY: help all fetch build release run run-release launch fmt format test clippy check \
	gtk-smoke-test install install-user uninstall uninstall-user package \
	package-install package-uninstall clean

help: ## Show available workflows
	@awk 'BEGIN {FS = ":.*## "} /^[a-zA-Z0-9_.-]+:.*## / {printf "%-22s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

all: build ## Build the app and server in debug mode

fetch: ## Fetch locked Rust dependencies (TARGET=... optionally selects a target)
	$(CARGO) fetch $(CARGO_FETCH_FLAGS) $(if $(TARGET),--target $(TARGET),)

build: ## Build the app and server in debug mode
	$(CARGO) build $(CARGO_BUILD_FLAGS) --workspace

release: ## Build all workspace targets in release mode
	$(CARGO) build $(CARGO_BUILD_FLAGS) --release --workspace

run: build ## Build and launch the debug app
	$(CARGO_TARGET_DIR)/debug/kulkan

run-release: release ## Build and launch the release app
	$(CARGO_TARGET_DIR)/release/kulkan

launch: ## Launch an installed Kulkán app from PATH
	kulkan

fmt: ## Check Rust formatting
	$(CARGO) fmt --all -- --check

format: ## Format Rust source files
	$(CARGO) fmt --all

test: ## Run the locked workspace test suite
	$(CARGO) test $(CARGO_TEST_FLAGS) --workspace

clippy: ## Run Clippy with warnings denied
	$(CARGO) clippy $(CARGO_CLIPPY_FLAGS) --workspace --all-targets -- -D warnings

check: fmt test clippy ## Run formatting, tests, and Clippy

gtk-smoke-test: ## Run the isolated headless GTK smoke test
	./scripts/gtk-smoke-test.sh

install: ## Install prebuilt release binaries and desktop assets (make release first)
	@test -x "$(CARGO_TARGET_DIR)/release/kulkan" || { echo "Missing release app; run 'make release' first." >&2; exit 1; }
	@test -x "$(CARGO_TARGET_DIR)/release/kulkan-server" || { echo "Missing release server; run 'make release' first." >&2; exit 1; }
	install -Dm0755 "$(CARGO_TARGET_DIR)/release/kulkan" "$(DESTDIR)$(BINDIR)/kulkan"
	install -Dm0755 "$(CARGO_TARGET_DIR)/release/kulkan-server" "$(DESTDIR)$(BINDIR)/kulkan-server"
	install -Dm0644 data/org.kulkan.Kulkan.desktop "$(DESTDIR)$(DATADIR)/applications/org.kulkan.Kulkan.desktop"
	install -Dm0644 data/icons/512x512/apps/org.kulkan.Kulkan.png "$(DESTDIR)$(DATADIR)/icons/hicolor/512x512/apps/org.kulkan.Kulkan.png"
	install -Dm0644 LICENSE "$(DESTDIR)$(DATADIR)/licenses/kulkan/LICENSE"

install-user: PREFIX := $(HOME)/.local
install-user: install ## Install to ~/.local without elevated privileges

uninstall: ## Remove files installed under PREFIX (default: /usr/local)
	rm -f "$(DESTDIR)$(BINDIR)/kulkan" \
		"$(DESTDIR)$(BINDIR)/kulkan-server" \
		"$(DESTDIR)$(DATADIR)/applications/org.kulkan.Kulkan.desktop" \
		"$(DESTDIR)$(DATADIR)/icons/hicolor/512x512/apps/org.kulkan.Kulkan.png" \
		"$(DESTDIR)$(DATADIR)/licenses/kulkan/LICENSE"

uninstall-user: PREFIX := $(HOME)/.local
uninstall-user: uninstall ## Remove files installed under ~/.local

package: ## Build the local Arch Linux package (run as a regular user)
	$(MAKEPKG) --syncdeps --cleanbuild

package-install: package ## Build and install the latest local Arch package
	@set -e; package_file="$$(find . -maxdepth 1 -type f -name 'kulkan-*.pkg.tar.zst' -printf '%T@ %f\n' | sort -nr | head -n 1 | cut -d ' ' -f 2-)"; \
	if [ -z "$$package_file" ]; then echo "No Kulkán package was produced." >&2; exit 1; fi; \
	$(SUDO) pacman -U "./$$package_file"

package-uninstall: ## Remove the installed Arch Linux package
	$(SUDO) pacman -Rns kulkan

clean: ## Remove Cargo build artifacts
	$(CARGO) clean
