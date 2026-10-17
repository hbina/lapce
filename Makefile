# Linux build helpers.
.PHONY: all help ubuntu-deps binary install clean
all: help
help:
	@echo "Targets: ubuntu-deps, binary, install, clean"
ubuntu-deps:
	apt-get update -y
	apt-get install -y clang libxkbcommon-x11-dev pkg-config libvulkan-dev libgtk-3-dev libwayland-dev xorg-dev libxcb-shape0-dev libxcb-xfixes0-dev
binary:
	cargo build --profile release-lto
install: binary
	install -Dm755 target/release-lto/lapce $(DESTDIR)$(PREFIX)/bin/lapce
	install -Dm755 target/release-lto/lapce-proxy $(DESTDIR)$(PREFIX)/bin/lapce-proxy
clean:
	cargo clean
PREFIX ?= /usr/local
