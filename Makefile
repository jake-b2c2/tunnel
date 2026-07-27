APP_NAME    := Tunnel
BUNDLE      := dist/$(APP_NAME).app
# Per-user install by default (no admin needed, works on managed Macs).
# Override for a system-wide install: sudo make install INSTALL_DIR=/Applications
INSTALL_DIR ?= $(HOME)/Applications
INSTALLED   := $(INSTALL_DIR)/$(APP_NAME).app
EXEC_MATCH  := $(APP_NAME).app/Contents/MacOS/tunnel
# A stray dev binary (target/release/tunnel) that a leftover LaunchAgent may keep
# alive, plus any LaunchAgents matching this glob. Older versions of this app
# shipped a `com.tunnel.*` LaunchAgent with KeepAlive=true; uninstall must tear
# that down or it relaunches the app on quit and survives removal of the .app.
DEV_EXEC    := target/release/tunnel
AGENT_GLOB  := $(HOME)/Library/LaunchAgents/com.tunnel*.plist

.PHONY: build install uninstall run icons clean help

help:
	@echo "Tunnel — macOS menu-bar service for b2c2 SSM tunnels"
	@echo ""
	@echo "  make build      Build the release binary and assemble $(BUNDLE)"
	@echo "  make install    Build and install to $(INSTALL_DIR), then launch"
	@echo "                  (override: sudo make install INSTALL_DIR=/Applications)"
	@echo "  make uninstall  Quit, remove login item + any leftover LaunchAgent,"
	@echo "                  delete $(INSTALLED)"
	@echo "  make run        Build and run from ./dist (for testing)"
	@echo "  make icons      Regenerate icons (needs librsvg: brew install librsvg)"
	@echo "  make clean      cargo clean + remove ./dist"
	@echo ""
	@echo "After installing: click the menu-bar icon -> Start."
	@echo "To auto-start at login: check 'Start at login' in the menu."

build:
	./bundle.sh

install: build
	@echo "==> Stopping any running instance"
	-@pkill -f "$(EXEC_MATCH)" 2>/dev/null || true
	@echo "==> Installing to $(INSTALL_DIR)"
	mkdir -p "$(INSTALL_DIR)"
	rm -rf "$(INSTALLED)"
	cp -R "$(BUNDLE)" "$(INSTALLED)"
	@echo "==> Launching"
	open "$(INSTALLED)"
	@echo ""
	@echo "Installed. Look for the tunnel icon in your menu bar, then click Start."
	@echo "Requires the 'b2c2' CLI on your PATH (and redis-cli/psql for health checks)."

uninstall:
	@echo "==> Removing any leftover LaunchAgent (older versions installed one)"
	-@for plist in $(AGENT_GLOB); do \
		[ -e "$$plist" ] || continue; \
		label=$$(basename "$$plist" .plist); \
		echo "    unloading $$label"; \
		launchctl bootout gui/$$(id -u)/$$label 2>/dev/null \
			|| launchctl unload "$$plist" 2>/dev/null || true; \
		rm -f "$$plist"; \
	done
	@echo "==> Stopping running instances"
	-@pkill -f "$(EXEC_MATCH)" 2>/dev/null || true
	-@pkill -f "$(DEV_EXEC)" 2>/dev/null || true
	-@osascript -e 'tell application "System Events" to delete login item "$(APP_NAME)"' 2>/dev/null || true
	rm -rf "$(INSTALLED)"
	@echo "Uninstalled $(APP_NAME)."

run: build
	open "$(BUNDLE)"

icons:
	./assets/gen-icons.sh

clean:
	cargo clean
	rm -rf dist
