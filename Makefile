BINARY := hotline

# Build stamp. Go's own VCS stamping is silently absent in this checkout — the
# repo is a git WORKTREE, so `.git` is a file rather than a directory and
# `debug.ReadBuildInfo()` reports no vcs.revision. Without this, a locally built
# or `go install`ed binary answers `hotline --version` with a bare "hotline dev"
# and the only way to tell which commit is deployed is comparing mtimes.
# goreleaser sets the same three symbols from the tag; these are the local twin.
VERSION ?= $(shell git describe --tags --match 'v[0-9]*' --always --dirty 2>/dev/null || echo dev)
COMMIT  ?= $(shell git rev-parse HEAD 2>/dev/null)
DATE    ?= $(shell date -u +%Y-%m-%dT%H:%M:%SZ)
LDFLAGS := -X main.version=$(VERSION) -X main.commit=$(COMMIT) -X main.date=$(DATE)

.DEFAULT_GOAL := build

# Build the binary into the repo (gitignored as /hotline).
.PHONY: build
build:
	go build -ldflags "$(LDFLAGS)" -o $(BINARY) ./cmd/hotline

# Install hotline system-wide to GOBIN (or $(go env GOPATH)/bin), on PATH.
# This is what a box's .mcp.json invokes as bare `hotline`.
.PHONY: install
install:
	go install -ldflags "$(LDFLAGS)" ./cmd/hotline

# Print the stamp this tree would bake in, without building anything.
.PHONY: version
version:
	@echo "version=$(VERSION) commit=$(COMMIT) date=$(DATE)"

# Formatting, vet, and the full race-enabled test suite — the pre-commit gate.
.PHONY: fmt
fmt:
	gofmt -w .

.PHONY: vet
vet:
	go vet ./...

.PHONY: test
test:
	go test -race ./...

.PHONY: check
check: fmt vet test

# Regenerate the pi extension's cross-language test goldens from the real
# binary (harness/pi/test/goldens.json). CI runs this and `git diff --exit-code`
# so a Go-side schema/instructions change that isn't regenerated fails the build.
.PHONY: goldens
goldens:
	node harness/pi/test/gen-goldens.mjs

.PHONY: clean
clean:
	rm -f $(BINARY)
