BINARY       := slaunch
RELEASE_BIN  := target/release/$(BINARY)

PREFIX       ?= $(HOME)/.local
BIN_DIR      := $(PREFIX)/bin
CONFIG_DIR   := $(HOME)/.config/$(BINARY)

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
	else \
		echo "Config already exists at $(CONFIG_DIR), skipping"; \
	fi
	@echo "Installed $(BINARY) to $(BIN_DIR)/$(BINARY)"

uninstall:
	rm -f $(BIN_DIR)/$(BINARY)
	@echo "Removed $(BIN_DIR)/$(BINARY)"
	@echo "Config at $(CONFIG_DIR) was left in place"
