# solara — build, run, install
#
#   make            build release binary + payload
#   make ui         build, then open the control panel
#   make run        build, then print the target list
#   make install    install to PREFIX (default ~/.local)
#   make uninstall  remove installed files
#   make doctor     preflight: ptrace, client, payload, flatpak
#   make test       selftest + inject-test
#   make dist       tarball in dist/

PREFIX      ?= $(HOME)/.local
BINDIR      ?= $(PREFIX)/bin
LIBDIR      ?= $(PREFIX)/lib/solara
APPDIR      ?= $(PREFIX)/share/applications
PORT        ?= 8787

CARGO       ?= cargo
CARGOFLAGS  ?= --release

BIN         := target/release/solara
PAYLOAD     := target/release/libpayload.so

SOURCES     := $(wildcard src/*.rs) $(wildcard src/ui_assets/*) \
               $(wildcard payload/src/*.rs)

.DEFAULT_GOAL := all
.PHONY: all release payload ui run install uninstall doctor test selftest \
        inject-test fmt clippy check clean dist help

all: release

# --- build -------------------------------------------------------------

release: $(BIN) $(PAYLOAD)

$(BIN): Cargo.toml payload/Cargo.toml $(SOURCES)
	$(CARGO) build $(CARGOFLAGS)

$(PAYLOAD): Cargo.toml payload/Cargo.toml payload/src/lib.rs
	$(CARGO) build $(CARGOFLAGS) -p payload

payload: $(PAYLOAD)

# --- run ---------------------------------------------------------------

ui: release
	./$(BIN) ui $(PORT)

run: release
	./$(BIN) find

# --- quality -----------------------------------------------------------

check: release
	./$(BIN) selftest

selftest: release
	./$(BIN) selftest

inject-test: release
	./$(BIN) inject-test

test: selftest inject-test

fmt:
	$(CARGO) fmt --all

clippy:
	$(CARGO) clippy --all-targets -- -D warnings

doctor:
	@./scripts/doctor.sh

# --- install -----------------------------------------------------------

install: release
	@install -d "$(DESTDIR)$(BINDIR)" "$(DESTDIR)$(LIBDIR)" "$(DESTDIR)$(APPDIR)"
	@install -m 755 "$(BIN)" "$(DESTDIR)$(BINDIR)/solara"
	@install -m 644 "$(PAYLOAD)" "$(DESTDIR)$(LIBDIR)/libpayload.so"
	@sed -e "s|@BINDIR@|$(BINDIR)|g" \
	     -e "s|@LIBDIR@|$(LIBDIR)|g" \
	     -e "s|@PORT@|$(PORT)|g" \
	     packaging/solara-ui.desktop > "$(DESTDIR)$(APPDIR)/solara-ui.desktop"
	@chmod 644 "$(DESTDIR)$(APPDIR)/solara-ui.desktop"
	@echo "installed:"
	@echo "  $(BINDIR)/solara"
	@echo "  $(LIBDIR)/libpayload.so"
	@echo "  $(APPDIR)/solara-ui.desktop"
	@echo
	@echo "make sure $(BINDIR) is on your PATH, then run: solara ui"

uninstall:
	@rm -f "$(DESTDIR)$(BINDIR)/solara"
	@rm -rf "$(DESTDIR)$(LIBDIR)"
	@rm -f "$(DESTDIR)$(APPDIR)/solara-ui.desktop"
	@echo "removed solara from $(DESTDIR)$(PREFIX)"

# --- packaging ---------------------------------------------------------

dist: release
	@mkdir -p dist
	@tar -czf dist/solara-linux-x86_64.tar.gz \
		-C . $(BIN) $(PAYLOAD) README.md
	@echo "dist/solara-linux-x86_64.tar.gz"

# --- housekeeping ------------------------------------------------------

clean:
	$(CARGO) clean
	@rm -rf dist

help:
	@sed -n '1,10p' Makefile | sed 's/^# \{0,1\}//'
