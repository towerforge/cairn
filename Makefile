# Cairn — entry point. Builds and runs on the host: it is a desktop app, it
# does not go in Docker. Releases are built by GitHub Actions
# (.github/workflows/release.yml) on native runners, from the tag that
# `make release` pushes.
SHELL := /bin/bash
.DEFAULT_GOAL := help

# ---------------------------------------------------------------------------
# Project
# ---------------------------------------------------------------------------

APP_NAME := cairn
DIST_DIR ?= dist

RESET := \033[0m
BOLD  := \033[1m
DIM   := \033[2m
GREEN := \033[0;32m
RED   := \033[0;31m

VERSION       ?= $(shell cat VERSION)
CARGO_VERSION := $(shell sed -n 's/^version[[:space:]]*=[[:space:]]*"\(.*\)"/\1/p' Cargo.toml | head -n1)

HOST_OS   := $(shell uname -s | tr '[:upper:]' '[:lower:]')
HOST_ARCH := $(shell uname -m)

# Branches of the release flow: work happens on $(DEV_BRANCH) and a release
# promotes it to $(MAIN_BRANCH), which is what the tag is cut from.
DEV_BRANCH  ?= dev
MAIN_BRANCH ?= main

# ---------------------------------------------------------------------------
# Build target — set CARGO_TARGET to a Rust triple to cross-compile.
#   macOS builds both architectures on a Mac (rustup target add <triple>).
#   Linux builds natively (the window needs the system's X11/Wayland
#   headers): CI uses an x86_64 and an arm64 runner.
# ---------------------------------------------------------------------------

CARGO_TARGET ?=

PLATFORM ?= $(strip $(if $(CARGO_TARGET),\
	$(if $(findstring apple-darwin,$(CARGO_TARGET)),macos,linux),\
	$(if $(filter darwin,$(HOST_OS)),macos,linux)))

BIN := $(if $(CARGO_TARGET),target/$(CARGO_TARGET)/release/$(APP_NAME),target/release/$(APP_NAME))

# Artifacts use the Rust arch name: uname says arm64 on Apple Silicon, Rust says aarch64
ARCH := $(subst arm64,aarch64,$(if $(CARGO_TARGET),$(word 1,$(subst -, ,$(CARGO_TARGET))),$(HOST_ARCH)))

# e.g. cairn-macos-aarch64.tar.gz (Cairn.app inside) · cairn-linux-x86_64.tar.gz
# (binary + launcher + icon). No version in the name: install.sh and
# `cairn update` build the URL from the platform alone.
ARTIFACT := $(APP_NAME)-$(PLATFORM)-$(ARCH).tar.gz
STAGE    := $(DIST_DIR)/.stage-$(PLATFORM)-$(ARCH)

MACOS_TARGETS ?= aarch64-apple-darwin x86_64-apple-darwin

APP_BUNDLE := $(DIST_DIR)/Cairn.app
DMG        := $(DIST_DIR)/Cairn-$(VERSION).dmg

# ---------------------------------------------------------------------------
# Phony targets
# ---------------------------------------------------------------------------

.PHONY: help run check fmt fmt-check clippy test build install uninstall \
        app bundle dmg install-app uninstall-app install-linux uninstall-linux \
        package package-all-macos checksums \
        version set-version write-version release clean dist-clean

# ---------------------------------------------------------------------------
# Help
# ---------------------------------------------------------------------------

help:
	@printf "\n  $(BOLD)$(APP_NAME)$(RESET)  $(DIM)v$(VERSION)$(RESET)\n\n"
	@echo "  Development"
	@echo "    make run                    Builds and opens the Cairn window (debug)"
	@echo "    make check                  fmt --check + clippy -D warnings + tests (what CI runs)"
	@echo "    make fmt                    Format the code"
	@echo "    make build                  Release binary for the host (or CARGO_TARGET)"
	@echo "    make install                cargo install from this checkout"
	@echo ""
	@echo "  Desktop app"
	@echo "    make app                    macOS: $(APP_BUNDLE)"
	@echo "    make dmg                    macOS: $(DMG)"
	@echo "    make install-app            macOS: installs Cairn.app in /Applications"
	@echo "    make install-linux          Linux: binary + launcher in ~/.local"
	@echo ""
	@echo "  Packaging"
	@echo "    make package                Build + package for the host (or CARGO_TARGET) into $(DIST_DIR)/"
	@echo "    make package-all-macos      Both macOS architectures (needs a Mac)"
	@echo "    make checksums              $(DIST_DIR)/checksums.txt (SHA-256)"
	@echo ""
	@echo "  Release"
	@echo "    make version                Show the version and flag drift between VERSION and Cargo.toml"
	@echo "    make set-version x.y.z      Write a new version to VERSION, Cargo.toml and Cargo.lock"
	@echo "    make release                $(DEV_BRANCH)→$(MAIN_BRANCH): bumps VERSION, merges, tags vX.Y.Z and pushes;"
	@echo "                                GitHub Actions builds every platform and publishes the release"
	@echo "                                from $(MAIN_BRANCH): tags the current version and pushes it"
	@echo ""
	@echo "  Utilities"
	@echo "    make clean                  cargo clean"
	@echo "    make dist-clean             Remove $(DIST_DIR)/"
	@echo ""

# ---------------------------------------------------------------------------
# Development
# ---------------------------------------------------------------------------

run:
	cargo run

check: fmt-check clippy test        ## lint + tests (yes, tests are included here)

fmt:
	cargo fmt

fmt-check:
	cargo fmt --check

clippy:
	cargo clippy --all-targets -- -D warnings

test:
	cargo test

build:
	cargo build --release --locked $(if $(CARGO_TARGET),--target $(CARGO_TARGET),)

install:
	cargo install --path . --locked

uninstall:
	cargo uninstall $(APP_NAME)

# ---------------------------------------------------------------------------
# Desktop app
# ---------------------------------------------------------------------------

app: build
	@if [ "$(PLATFORM)" != "macos" ]; then echo "make app is macOS-only (on Linux: make install-linux)"; exit 1; fi
	@$(MAKE) --no-print-directory bundle BUNDLE_DIR="$(APP_BUNDLE)"
	@printf "  $(GREEN)✓$(RESET) $(APP_BUNDLE)\n"

# Cairn.app around $(BIN), signed ad hoc. BUNDLE_DIR says where.
bundle:
	@rm -rf "$(BUNDLE_DIR)"
	@mkdir -p "$(BUNDLE_DIR)/Contents/MacOS" "$(BUNDLE_DIR)/Contents/Resources"
	@cp "$(BIN)" "$(BUNDLE_DIR)/Contents/MacOS/$(APP_NAME)"
	@cp assets/icons/icon.icns "$(BUNDLE_DIR)/Contents/Resources/icon.icns"
	@sed 's/@VERSION@/$(VERSION)/g' assets/macos/Info.plist > "$(BUNDLE_DIR)/Contents/Info.plist"
	@codesign --force --deep --sign - "$(BUNDLE_DIR)" 2>/dev/null

dmg: app
	@rm -rf "$(DIST_DIR)/dmg" "$(DMG)"
	@mkdir -p "$(DIST_DIR)/dmg"
	@cp -R "$(APP_BUNDLE)" "$(DIST_DIR)/dmg/"
	@ln -s /Applications "$(DIST_DIR)/dmg/Applications"
	@hdiutil create -quiet -volname "Cairn $(VERSION)" -srcfolder "$(DIST_DIR)/dmg" -ov -format UDZO "$(DMG)"
	@rm -rf "$(DIST_DIR)/dmg"
	@printf "  $(GREEN)✓$(RESET) $(DMG)\n"

install-app: app
	@rm -rf /Applications/Cairn.app
	@cp -R "$(APP_BUNDLE)" /Applications/
	@printf "  $(GREEN)✓$(RESET) /Applications/Cairn.app\n"

uninstall-app:
	rm -rf /Applications/Cairn.app

install-linux: build
	install -Dm755 "$(BIN)" "$(HOME)/.local/bin/cairn"
	install -Dm644 assets/icons/128x128.png "$(HOME)/.local/share/icons/hicolor/128x128/apps/cairn.png"
	install -Dm644 assets/linux/cairn.desktop "$(HOME)/.local/share/applications/cairn.desktop"
	-update-desktop-database "$(HOME)/.local/share/applications" 2>/dev/null

uninstall-linux:
	rm -f "$(HOME)/.local/bin/cairn" \
	      "$(HOME)/.local/share/icons/hicolor/128x128/apps/cairn.png" \
	      "$(HOME)/.local/share/applications/cairn.desktop"

# ---------------------------------------------------------------------------
# Packaging — dist/cairn-<platform>-<arch>.tar.gz
#   macOS: Cairn.app
#   Linux: cairn + share/applications/cairn.desktop + share/icons/…/cairn.png
# ---------------------------------------------------------------------------

package: build
	@rm -rf "$(STAGE)" && mkdir -p "$(STAGE)" "$(DIST_DIR)"
	@rm -f "$(DIST_DIR)/$(ARTIFACT)"
	@if [ "$(PLATFORM)" = "macos" ]; then \
		$(MAKE) --no-print-directory bundle BUNDLE_DIR="$(STAGE)/Cairn.app"; \
		COPYFILE_DISABLE=1 tar -czf "$(DIST_DIR)/$(ARTIFACT)" -C "$(STAGE)" Cairn.app; \
	else \
		install -Dm755 "$(BIN)" "$(STAGE)/$(APP_NAME)"; \
		install -Dm644 assets/linux/cairn.desktop "$(STAGE)/share/applications/cairn.desktop"; \
		install -Dm644 assets/icons/128x128.png "$(STAGE)/share/icons/hicolor/128x128/apps/cairn.png"; \
		tar -czf "$(DIST_DIR)/$(ARTIFACT)" -C "$(STAGE)" $(APP_NAME) share; \
	fi
	@rm -rf "$(STAGE)"
	@echo "$(DIST_DIR)/$(ARTIFACT)"

package-all-macos:
	@if [ "$(HOST_OS)" != "darwin" ]; then \
		echo "ERROR: macOS targets can only be built on a Mac"; exit 1; \
	fi
	@set -e; \
	for target in $(MACOS_TARGETS); do \
		echo "==> [macos] $$target"; \
		rustup target add $$target >/dev/null; \
		$(MAKE) --no-print-directory package CARGO_TARGET=$$target; \
	done

checksums:
	@cd "$(DIST_DIR)" 2>/dev/null || { echo "ERROR: $(DIST_DIR) does not exist"; exit 1; }; \
	shopt -s nullglob; files=$$(echo *.tar.gz); \
	if [ -z "$$files" ]; then echo "ERROR: no .tar.gz files found in $(DIST_DIR)"; exit 1; fi; \
	{ if command -v sha256sum >/dev/null 2>&1; then sha256sum $$files; else shasum -a 256 $$files; fi; } > checksums.txt; \
	echo "$(DIST_DIR)/checksums.txt"

# ---------------------------------------------------------------------------
# Version
#   make version              Show the version and flag drift between VERSION
#                             and Cargo.toml
#   make set-version x.y.z    Write a new semver version to VERSION, Cargo.toml
#                             and Cargo.lock (also accepts NEW=x.y.z)
#   make release              $(DEV_BRANCH)→$(MAIN_BRANCH): bump, merge, tag and push
# ---------------------------------------------------------------------------

# Accept a bare positional argument:  make set-version 0.4.0
# Without this Make would treat "0.4.0" as another target.
ifeq (set-version,$(firstword $(MAKECMDGOALS)))
  SET_VERSION_POS := $(wordlist 2,$(words $(MAKECMDGOALS)),$(MAKECMDGOALS))
  ifneq ($(SET_VERSION_POS),)
    $(eval $(SET_VERSION_POS):;@:)
    NEW ?= $(firstword $(SET_VERSION_POS))
  endif
endif

version:
	@printf "\n  $(BOLD)$(APP_NAME)$(RESET)  $(GREEN)v$(VERSION)$(RESET)\n"
	@printf "  $(DIM)VERSION      → $(VERSION)$(RESET)\n"
	@printf "  $(DIM)Cargo.toml   → $(CARGO_VERSION)$(RESET)\n"
	@if [ "$(VERSION)" != "$(CARGO_VERSION)" ]; then \
		printf "\n  $(BOLD)⚠ drift$(RESET)  $(DIM)Cargo.toml is out of sync — run:$(RESET)  make set-version $(VERSION)\n"; \
	fi
	@printf "\n  $(DIM)bump:$(RESET)  make set-version x.y.z\n\n"

set-version:
	@if [ -z "$(NEW)" ]; then \
		printf "\n  $(BOLD)error$(RESET)  missing version  $(DIM)(usage: make set-version 0.4.0)$(RESET)\n\n"; exit 1; \
	fi
	@if ! echo "$(NEW)" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$$'; then \
		printf "\n  $(BOLD)error$(RESET)  '$(NEW)' is not semver (x.y.z)\n\n"; exit 1; \
	fi
	@old="$(VERSION)"; new="$(NEW)"; \
	if [ "$$old" = "$$new" ] && [ "$(CARGO_VERSION)" = "$$new" ]; then \
		printf "\n  $(DIM)already at v$$new — nothing to do$(RESET)\n\n"; exit 0; \
	fi; \
	printf "\n  $(BOLD)bump$(RESET)  $(DIM)v$$old$(RESET)  →  $(GREEN)v$$new$(RESET)\n\n"; \
	$(MAKE) --no-print-directory write-version NEW="$$new"; \
	printf "  $(DIM)updated:$(RESET)\n"; \
	printf "    VERSION      → $$new\n"; \
	printf "    Cargo.toml   → version = \"$$new\"\n"; \
	printf "    Cargo.lock   → synced\n"; \
	printf "\n  $(DIM)review with$(RESET)  git diff  $(DIM)then commit and run$(RESET)  make release\n\n"

# The three files that carry the version, written in one place: `set-version`
# and `release` both go through here.
write-version:
	@printf "%s\n" "$(NEW)" > VERSION
	@awk -v new="$(NEW)" 'BEGIN{done=0} !done && /^version[[:space:]]*=[[:space:]]*"[^"]+"/ {sub(/"[^"]+"/, "\"" new "\""); done=1} {print}' Cargo.toml > Cargo.toml.tmp && mv Cargo.toml.tmp Cargo.toml
	@cargo update --workspace --offline --quiet

## Publishes a release along the fixed chain dev → main: the target branch is
## decided by the branch you are on, without asking.
##
##   · from dev  → main   Proposes the next version (patch/minor/major or one
##                        you type), writes it to VERSION, Cargo.toml and
##                        Cargo.lock, commits it on dev, merges dev into main,
##                        tags vX.Y.Z and pushes. CI builds and publishes.
##   · from main          Promotion: keeps the version, tags it if the tag is
##                        not there yet, and pushes.
##   · another branch     Refused: a release is cut from dev or main.
##
## If the merge collides only in VERSION it is resolved with the release
## version; any other conflict, Cargo.toml included, aborts the merge and
## leaves everything as it was, back on the branch you started from.
release:
	@set -e; \
	if [ -n "$$(git status --porcelain -uno)" ]; then \
		printf "\n  $(RED)✗ Uncommitted changes — commit or stash before releasing.$(RESET)\n\n"; exit 1; \
	fi; \
	if ! git remote get-url origin >/dev/null 2>&1; then \
		printf "\n  $(RED)✗ No 'origin' remote to push to.$(RESET)\n\n"; exit 1; \
	fi; \
	V=$$(cat VERSION 2>/dev/null | tr -d '[:space:]'); \
	if [ "$$V" != "$(CARGO_VERSION)" ]; then \
		printf "\n  $(RED)✗ VERSION ($$V) and Cargo.toml ($(CARGO_VERSION)) differ$(RESET)  $(DIM)run: make set-version $$V$(RESET)\n\n"; exit 1; \
	fi; \
	ORIG=$$(git rev-parse --abbrev-ref HEAD); \
	case "$$ORIG" in \
		$(DEV_BRANCH))  TARGET=$(MAIN_BRANCH); MODE=bump ;; \
		$(MAIN_BRANCH)) TARGET=$(MAIN_BRANCH); MODE=promo ;; \
		*) printf "\n  $(RED)✗ A release is cut from $(DEV_BRANCH) or $(MAIN_BRANCH), not from '$$ORIG'$(RESET)\n\n"; exit 1 ;; \
	esac; \
	if ! git rev-parse -q --verify "refs/heads/$$TARGET" >/dev/null; then \
		if git ls-remote --exit-code --heads origin "$$TARGET" >/dev/null 2>&1; then \
			printf "\n  $(DIM)$$TARGET is not here but it is on origin — creating it...$(RESET)\n"; \
			git fetch -q origin "$$TARGET:$$TARGET"; \
		else \
			printf "\n  $(RED)✗ Branch '$$TARGET' is neither here nor on origin$(RESET)\n\n"; exit 1; \
		fi; \
	fi; \
	CI_MSG="GitHub Actions builds macOS (Cairn.app) and Linux, writes checksums.txt and publishes the GitHub release"; \
	if [ "$$MODE" = "promo" ]; then \
		printf "\n  $(BOLD)Release · promotion$(RESET)  $(DIM)$$TARGET · version $$V$(RESET)\n\n"; \
		if git rev-parse -q --verify "refs/tags/v$$V" >/dev/null; then \
			printf "  $(RED)✗ Tag v$$V already exists$(RESET)  $(DIM)bump from $(DEV_BRANCH) instead$(RESET)\n\n"; exit 1; \
		fi; \
		printf "  $(BOLD)Summary$(RESET)\n\n"; \
		printf "    Version  $(BOLD)$$V$(RESET)  $(DIM)kept, no bump$(RESET)\n"; \
		printf "    Tag      v$$V\n"; \
		printf "    Push     origin $$TARGET · origin v$$V\n"; \
		printf "    CI       $(DIM)$$CI_MSG$(RESET)\n"; \
		printf "\n  Continue? [y/N]: "; read ANS; echo ""; \
		if [ "$$ANS" != "y" ]; then \
			printf "  $(DIM)Cancelled — nothing was touched.$(RESET)\n\n"; exit 0; \
		fi; \
		printf "  $(DIM)→ Tagging v$$V$(RESET)\n"; \
		git tag -a "v$$V" -m "Release v$$V"; \
		printf "  $(DIM)→ Publishing $$TARGET and the tag$(RESET)\n"; \
		git push -q origin "$$TARGET"; \
		git push -q origin "v$$V"; \
		printf "\n  $(GREEN)✓ v$$V published from $$TARGET$(RESET)\n\n"; \
		exit 0; \
	fi; \
	IFS=. read MA MI PA <<< "$${V:-0.0.0}"; \
	P_PATCH="$$MA.$$MI.$$((PA+1))"; \
	P_MINOR="$$MA.$$((MI+1)).0"; \
	P_MAJOR="$$((MA+1)).0.0"; \
	printf "\n  $(BOLD)Release$(RESET)  $(DIM)$$ORIG → $$TARGET · current version $$V$(RESET)\n\n"; \
	printf "  $(BOLD)New version$(RESET)\n\n"; \
	printf "    $(BOLD)1$(RESET)  $$P_PATCH  $(DIM)patch$(RESET)\n"; \
	printf "    $(BOLD)2$(RESET)  $$P_MINOR  $(DIM)minor$(RESET)\n"; \
	printf "    $(BOLD)3$(RESET)  $$P_MAJOR  $(DIM)major$(RESET)\n"; \
	printf "    $(DIM)or type the version by hand (X.Y.Z)$(RESET)\n\n"; \
	printf "  > "; read CH; echo ""; \
	case "$$CH" in \
		1) NEW_V=$$P_PATCH ;; \
		2) NEW_V=$$P_MINOR ;; \
		3) NEW_V=$$P_MAJOR ;; \
		*) NEW_V=$$CH ;; \
	esac; \
	if ! [[ "$$NEW_V" =~ ^[0-9]+\.[0-9]+\.[0-9]+$$ ]]; then \
		printf "  $(RED)✗ Invalid version: '$$NEW_V'$(RESET)  $(DIM)expected X.Y.Z, e.g. 0.4.0$(RESET)\n\n"; exit 1; \
	fi; \
	if [ "$$NEW_V" = "$$V" ]; then \
		printf "  $(RED)✗ The new version is the current one ($$V)$(RESET)\n\n"; exit 1; \
	fi; \
	if git rev-parse -q --verify "refs/tags/v$$NEW_V" >/dev/null; then \
		printf "  $(RED)✗ Tag v$$NEW_V already exists$(RESET)\n\n"; exit 1; \
	fi; \
	printf "  $(BOLD)Summary$(RESET)\n\n"; \
	printf "    Version  $(DIM)$$V →$(RESET) $(BOLD)$$NEW_V$(RESET)  $(DIM)VERSION · Cargo.toml · Cargo.lock$(RESET)\n"; \
	printf "    Merge    $$ORIG → $$TARGET\n"; \
	printf "    Tag      v$$NEW_V\n"; \
	printf "    Push     origin $$ORIG · origin $$TARGET · origin v$$NEW_V\n"; \
	printf "    CI       $(DIM)$$CI_MSG$(RESET)\n"; \
	printf "\n  Continue? [y/N]: "; read ANS; echo ""; \
	if [ "$$ANS" != "y" ]; then \
		printf "  $(DIM)Cancelled — nothing was touched.$(RESET)\n\n"; exit 0; \
	fi; \
	printf "  $(DIM)→ VERSION $$V → $$NEW_V · commit and push on $$ORIG$(RESET)\n"; \
	$(MAKE) --no-print-directory write-version NEW="$$NEW_V"; \
	git add VERSION Cargo.toml Cargo.lock; \
	git commit -q -m "chore: bump version to $$NEW_V"; \
	git push -q origin "$$ORIG"; \
	printf "  $(DIM)→ Merging $$ORIG into $$TARGET$(RESET)\n"; \
	git checkout -q "$$TARGET"; \
	if git ls-remote --exit-code --heads origin "$$TARGET" >/dev/null 2>&1; then \
		git pull -q --ff-only origin "$$TARGET"; \
	fi; \
	if ! git merge -q --no-ff "$$ORIG" -m "release: v$$NEW_V"; then \
		CONFLICTS=$$(git diff --name-only --diff-filter=U); \
		if [ "$$CONFLICTS" = "VERSION" ]; then \
			printf "  $(DIM)→ Conflict in VERSION — resolved with $$NEW_V$(RESET)\n"; \
			printf "%s\n" "$$NEW_V" > VERSION; \
			git add VERSION; \
			git commit -q --no-edit; \
		else \
			printf "\n  $(RED)✗ Merge conflicts in:$(RESET)\n"; \
			echo "$$CONFLICTS" | sed 's/^/      /'; \
			git merge --abort; \
			git checkout -q "$$ORIG"; \
			printf "\n  $(DIM)Merge aborted — the bump is committed on $$ORIG but nothing was published. Back on $$ORIG.$(RESET)\n\n"; \
			exit 1; \
		fi; \
	fi; \
	printf "  $(DIM)→ Tagging v$$NEW_V$(RESET)\n"; \
	git tag -a "v$$NEW_V" -m "Release v$$NEW_V"; \
	printf "  $(DIM)→ Publishing $$TARGET and the tag$(RESET)\n"; \
	git push -q origin "$$TARGET"; \
	git push -q origin "v$$NEW_V"; \
	git checkout -q "$$ORIG"; \
	echo ""; \
	printf "  $(GREEN)✓ Release v$$NEW_V published on $$TARGET$(RESET)\n"; \
	printf "\n  Back on $(BOLD)$$ORIG$(RESET)\n\n"

# ---------------------------------------------------------------------------
# Utilities
# ---------------------------------------------------------------------------

clean:
	cargo clean

dist-clean:
	@if [ -z "$(DIST_DIR)" ] || [ "$(DIST_DIR)" = "/" ] || [ "$(DIST_DIR)" = "." ]; then \
		echo "ERROR: refusing to remove DIST_DIR='$(DIST_DIR)'"; exit 1; \
	fi
	rm -rf "$(DIST_DIR)"
