BINARY := hotline

.DEFAULT_GOAL := build

# Build the binary into the repo (gitignored as /hotline).
.PHONY: build
build:
	go build -o $(BINARY) .

# Install hotline system-wide to GOBIN (or $(go env GOPATH)/bin), on PATH.
# This is what beastie-boy / rh-agent .mcp.json files invoke as bare `hotline`.
.PHONY: install
install:
	go install .

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

.PHONY: clean
clean:
	rm -f $(BINARY)
