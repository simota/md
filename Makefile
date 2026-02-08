BINARY := md
PKG := ./cmd/md

GO ?= go

.PHONY: test build install clean dist

test:
	$(GO) test ./...

build:
	CGO_ENABLED=0 $(GO) build -trimpath -ldflags="-s -w" -o $(BINARY) $(PKG)

install:
	CGO_ENABLED=0 $(GO) install $(PKG)

clean:
	rm -rf dist $(BINARY)

# Local packaging helper (mirrors GitHub Actions naming).
# Usage:
#   make dist VERSION=v0.1.0
dist: clean
	@if [ -z "$(VERSION)" ]; then echo "VERSION is required (e.g. VERSION=v0.1.0)"; exit 2; fi
	@mkdir -p dist
	@set -e; \
	for target in \
		"linux/amd64/tar.gz" \
		"linux/arm64/tar.gz" \
		"linux/armv7/tar.gz" \
		"darwin/amd64/tar.gz" \
		"darwin/arm64/tar.gz" \
		"windows/amd64/zip" \
		"windows/arm64/zip" \
	; do \
		GOOS="$${target%%/*}"; rest="$${target#*/}"; \
		GOARCH="$${rest%%/*}"; rest="$${rest#*/}"; \
		ARCHIVE="$${rest}"; \
		GOARM=""; ARCH="$${GOARCH}"; \
		if [ "$${GOARCH}" = "armv7" ]; then GOARCH="arm"; GOARM="7"; ARCH="armv7"; fi; \
		OUT="dist/$(BINARY)_$(VERSION)_$${GOOS}_$${ARCH}"; \
		echo "==> $${GOOS}/$${ARCH}"; \
		BIN="$(BINARY)"; if [ "$${GOOS}" = "windows" ]; then BIN="$(BINARY).exe"; fi; \
		CGO_ENABLED=0 GOOS="$${GOOS}" GOARCH="$${GOARCH}" GOARM="$${GOARM}" \
			$(GO) build -trimpath -ldflags="-s -w" -o "dist/$${BIN}" $(PKG); \
		if [ "$${GOOS}" = "windows" ]; then \
			(cd dist && zip -9 "../$${OUT}.zip" "$${BIN}"); \
		else \
			(cd dist && tar -czf "../$${OUT}.tar.gz" "$${BIN}"); \
		fi; \
		rm -f "dist/$${BIN}"; \
	done
	@cd dist && shasum -a 256 $(BINARY)_$(VERSION)_* > $(BINARY)_$(VERSION)_checksums.txt

