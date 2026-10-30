# Makefile for building Windows .exe from Linux

.PHONY: windows clean-patches icon build clean help dev-install dev-uninstall

# Local development install prefix and build profile.
#   make dev-install                 # release build -> ~/.local
#   make dev-install PROFILE=debug   # debug build
#   make dev-install PREFIX=/usr/local  # system-wide (needs write perms)
PREFIX ?= $(HOME)/.local
PROFILE ?= release

ifeq ($(PROFILE),release)
CARGO_FLAGS := --release
DEV_BIN := target/release/dicto
else
CARGO_FLAGS :=
DEV_BIN := target/debug/dicto
endif

# Default target
windows: patch-gpui icon build
	@echo ""
	@echo "=== Build successful ==="
	@echo "Output: target/x86_64-pc-windows-msvc/debug/dicto.exe"
	@ls -lh target/x86_64-pc-windows-msvc/debug/dicto.exe | awk '{print "Size:   " $$5}'

# Apply gpui patches (fixes cross-compilation issues)
patch-gpui:
	@echo "Applying gpui patches..."
	@bash patch-gpui-build.sh || true
	@echo "Done"

# Generate Windows icon from SVG
icon:
	@echo "Generating Windows icon..."
	@printf '%s\n%s\n' "32512 ICON \"icon.ico\"" "// Windows resource script" > gpui/icon.rc
	@if ! command -v rsvg-convert >/dev/null 2>&1; then \
		echo "ERROR: rsvg-convert is required for transparent icons (librsvg)." >&2; \
		exit 1; \
	fi
	@rsvg-convert -w 256 -h 256 -f png assets/icon.svg -o tmp_256.png
	@rsvg-convert -w 128 -h 128 -f png assets/icon.svg -o tmp_128.png
	@rsvg-convert -w 64 -h 64 -f png assets/icon.svg -o tmp_64.png
	@rsvg-convert -w 32 -h 32 -f png assets/icon.svg -o tmp_32.png
	@rsvg-convert -w 16 -h 16 -f png assets/icon.svg -o tmp_16.png
	@magick tmp_256.png tmp_128.png tmp_64.png tmp_32.png tmp_16.png -alpha on -define icon:auto-resize=256,128,64,32,16 gpui/icon.ico
	@rm -f tmp_*.png
	@file gpui/icon.ico | cut -d: -f2- | head -c 80

# Build Windows .exe
build:
	@echo "Building Windows .exe..."
	@cargo clean -p dicto --target x86_64-pc-windows-msvc
	@cargo xwin build --target x86_64-pc-windows-msvc -p dicto

# Build the current tree and install it for the local user (or PREFIX).
dev-install:
	@echo "Building dicto ($(PROFILE))..."
	@cargo build -p dicto $(CARGO_FLAGS)
	@echo "Installing to $(PREFIX)..."
	@install -Dm755 $(DEV_BIN) $(PREFIX)/bin/dicto
	@install -Dm644 assets/icon.svg $(PREFIX)/share/icons/hicolor/scalable/apps/dicto.svg
	@mkdir -p $(PREFIX)/share/applications
	@sed 's|^Exec=.*|Exec=$(PREFIX)/bin/dicto|' packaging/arch/dicto.desktop \
		> $(PREFIX)/share/applications/dicto.desktop
	@chmod 644 $(PREFIX)/share/applications/dicto.desktop
	@-update-desktop-database $(PREFIX)/share/applications 2>/dev/null || true
	@-gtk-update-icon-cache -qtf $(PREFIX)/share/icons/hicolor 2>/dev/null || true
	@echo "Installed: $(PREFIX)/bin/dicto"
	@echo "Run 'dicto' or launch 'Dicto' from your app menu."

# Remove the development install (does not touch build artifacts).
dev-uninstall:
	@rm -f $(PREFIX)/bin/dicto \
		$(PREFIX)/share/applications/dicto.desktop \
		$(PREFIX)/share/icons/hicolor/scalable/apps/dicto.svg
	@-update-desktop-database $(PREFIX)/share/applications 2>/dev/null || true
	@echo "Removed dicto from $(PREFIX)"

# Clean build artifacts
clean:
	@echo "Cleaning..."
	@rm -f gpui/icon.rc gpui/icon.ico
	@cargo clean -p dicto --target x86_64-pc-windows-msvc
	@echo "Cleaned Windows build artifacts"

# Reset gpui patches (useful after cargo update)
reset-patches:
	@echo "Resetting gpui patches..."
	@bash -c 'cd $$(find ~/.cargo/git/checkouts/zed-*/ae47ec9 -maxdepth 0 -type d 2>/dev/null | head -1) && git checkout -- crates/gpui/build.rs crates/gpui_windows/build.rs crates/gpui_windows/src/directx_renderer.rs'
	@echo "Patches reset. Run 'make windows' to apply and rebuild."

# Show help
help:
	@echo "Available targets:"
	@echo "  make windows      - Build Windows .exe (default)"
	@echo "  make patch-gpui   - Apply gpui cross-compilation patches only"
	@echo "  make icon         - Generate Windows icon only"
	@echo "  make build        - Build .exe (requires patches and icon)"
	@echo "  make clean        - Clean Windows build artifacts"
	@echo "  make reset-patches- Reset gpui patches (use after cargo update)"
	@echo "  make dev-install  - Build locally and install to ~/.local (PREFIX, PROFILE=debug)"
	@echo "  make dev-uninstall- Remove the local development install"
	@echo "  make help         - Show this help"
	@echo ""
	@echo "Example workflow:"
	@echo "  1. make windows           # Full build (patches + icon + compile)"
	@echo "  2. make icon && make build  # Generate icon and build separately"