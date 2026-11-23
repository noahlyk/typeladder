# Local builds. The version comes from git (see build.rs):
#   tag v1.2.0 checked out       -> 1.2.0
#   8 commits past v1.2.0        -> 1.2.0+8.g0587d41
#   uncommitted changes          -> ...+dirty
#   no tag yet                   -> 0.1.0+g0587d41
# Release builds set TYPELADDER_VERSION explicitly, so they keep the plain release version.

PREFIX ?= /usr/local
BIN := target/release/typeladder

.PHONY: all build install uninstall version clean check pkg

all: build

build:
	cargo build --release --locked

install: build
	install -Dm755 $(BIN) $(DESTDIR)$(PREFIX)/bin/typeladder
	install -Dm644 LICENSE $(DESTDIR)$(PREFIX)/share/licenses/typeladder/LICENSE
	install -Dm644 NOTICE $(DESTDIR)$(PREFIX)/share/licenses/typeladder/NOTICE
	install -Dm644 README.md $(DESTDIR)$(PREFIX)/share/doc/typeladder/README.md
	install -Dm644 docs/config.md $(DESTDIR)$(PREFIX)/share/doc/typeladder/config.md
	@echo "installed typeladder $$($(DESTDIR)$(PREFIX)/bin/typeladder --version | cut -d' ' -f2) to $(DESTDIR)$(PREFIX)/bin/typeladder"

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/typeladder
	rm -rf $(DESTDIR)$(PREFIX)/share/licenses/typeladder $(DESTDIR)$(PREFIX)/share/doc/typeladder

version: build
	@$(BIN) --version

clean:
	cargo clean

# What CI runs.
check:
	cargo fmt --all --check
	cargo clippy --release --all-targets --locked -- -D warnings
	cargo test --release --locked -- --test-threads=4

# An Arch package of this checkout into dist/. makepkg's work folders go under target/ because
# its default, src/, is this repository's own.
pkg:
	BUILDDIR=$(CURDIR)/target/makepkg PKGDEST=$(CURDIR)/dist makepkg -f
