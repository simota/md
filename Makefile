BINARY := md
CARGO ?= cargo

.PHONY: test build install clean dist

test:
	$(CARGO) test

build:
	$(CARGO) build --release

install:
	$(CARGO) install --path .

clean:
	$(CARGO) clean
	rm -rf dist

# Local packaging helper (mirrors GitHub Actions naming).
# Usage:
#   make dist VERSION=v0.1.0
dist:
	@if [ -z "$(VERSION)" ]; then echo "VERSION is required (e.g. VERSION=v0.1.0)"; exit 2; fi
	@mkdir -p dist
	@set -e; \
	for target in \
		"x86_64-unknown-linux-gnu/linux/amd64/tar.gz" \
		"aarch64-unknown-linux-gnu/linux/arm64/tar.gz" \
		"x86_64-apple-darwin/darwin/amd64/tar.gz" \
		"aarch64-apple-darwin/darwin/arm64/tar.gz" \
		"x86_64-pc-windows-msvc/windows/amd64/zip" \
		"aarch64-pc-windows-msvc/windows/arm64/zip" \
	; do \
		TARGET="$${target%%/*}"; rest="$${target#*/}"; \
		GOOS="$${rest%%/*}"; rest="$${rest#*/}"; \
		ARCH="$${rest%%/*}"; rest="$${rest#*/}"; \
		ARCHIVE="$${rest}"; \
		OUT="dist/$(BINARY)_$(VERSION)_$${GOOS}_$${ARCH}"; \
		echo "==> $${TARGET}"; \
		BIN="$(BINARY)"; if [ "$${GOOS}" = "windows" ]; then BIN="$(BINARY).exe"; fi; \
		$(CARGO) build --release --target "$${TARGET}"; \
		cp "target/$${TARGET}/release/$${BIN}" "dist/$${BIN}"; \
		if [ "$${GOOS}" = "windows" ]; then \
			(cd dist && zip -9 "../$${OUT}.zip" "$${BIN}"); \
		else \
			(cd dist && tar -czf "../$${OUT}.tar.gz" "$${BIN}"); \
		fi; \
		rm -f "dist/$${BIN}"; \
	done
	@cd dist && shasum -a 256 $(BINARY)_$(VERSION)_* > $(BINARY)_$(VERSION)_checksums.txt
