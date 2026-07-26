BINARY       := slaunch
RELEASE_BIN  := target/release/$(BINARY)

PREFIX       ?= /usr/local
BIN_DIR      := $(PREFIX)/bin

# $(PREFIX) defaults to /usr/local, so `make install` normally runs under sudo —
# where $(HOME) is root's. Config belongs to the invoking user, so resolve their
# real home from $(SUDO_USER) and hand them ownership of what we seed.
REAL_USER    := $(if $(SUDO_USER),$(SUDO_USER),$(USER))
REAL_HOME    := $(if $(SUDO_USER),$(shell getent passwd $(SUDO_USER) | cut -d: -f6),$(HOME))
CONFIG_DIR   := $(REAL_HOME)/.config/$(BINARY)

.PHONY: all build install uninstall

all: build

build:
	cargo build --release

install: build
	install -Dm755 $(RELEASE_BIN) $(BIN_DIR)/$(BINARY)
	@if [ ! -d $(CONFIG_DIR) ]; then \
		echo "Installing default config to $(CONFIG_DIR)"; \
		install -Dm644 config/config.toml $(CONFIG_DIR)/config.toml; \
		install -Dm644 config/style.css    $(CONFIG_DIR)/style.css; \
		chown -R $(REAL_USER) $(CONFIG_DIR); \
	else \
		echo "Config already exists at $(CONFIG_DIR), skipping"; \
	fi
	@echo "Installed $(BINARY) to $(BIN_DIR)/$(BINARY)"

uninstall:
	rm -f $(BIN_DIR)/$(BINARY)
	@echo "Removed $(BIN_DIR)/$(BINARY)"
	@echo "Config at $(CONFIG_DIR) was left in place"
