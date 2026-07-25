#!/bin/sh
#
# hotline installer.  Served verbatim at https://hotline.dev/install.sh
#
#     curl -fsSL https://hotline.dev/install.sh | sh
#
# What it does, in order: work out your OS and CPU, resolve a release, download
# that release's tarball and its checksums.txt, verify the tarball's SHA-256
# against checksums.txt, verify the signature ON checksums.txt, and only then
# move one binary into ~/.local/bin.
#
# What it does not do: no sudo, no package manager, no shell profile edits, no
# second script fetched and eval'd, no telemetry. Nothing is written outside the
# install directory, and nothing is written at all until every check has passed.
#
# Knobs, all optional:
#     HOTLINE_VERSION       release to install, e.g. v0.11.0   (default: latest)
#     HOTLINE_INSTALL_DIR   where the binary lands             (default: ~/.local/bin)
#     HOTLINE_RELEASES_URL  releases base, for a mirror        (default: GitHub)
#
# Re-running it upgrades in place. You are reading it before running it, which
# is the whole reason it is this short.

set -eu

REPO='1broseidon/hotline'
RELEASES="${HOTLINE_RELEASES_URL:-https://github.com/${REPO}/releases}"
DEFAULT_RELEASES="https://github.com/${REPO}/releases"

# Keyless cosign. There is no public key to distribute and no private key to
# leak: the signer identity IS a tagged run of this repo's release workflow,
# attested by GitHub's OIDC issuer and logged in Sigstore's public transparency
# log. Both values below are the trust anchor -- if you edit them, you are
# choosing to trust someone else.
COSIGN_ISSUER='https://token.actions.githubusercontent.com'
COSIGN_IDENTITY="^https://github\.com/${REPO}/\.github/workflows/release\.yml@refs/tags/v"

TMP=''

die() {
	printf '\nhotline install: %s\n\n' "$*" >&2
	exit 1
}

say() {
	printf '%s\n' "$*"
}

warn_loudly() {
	printf '\n' >&2
	printf '  !!  %s\n' "$@" >&2
	printf '\n' >&2
}

cleanup() {
	if [ -n "$TMP" ]; then
		rm -rf "$TMP"
	fi
}

# The goreleaser archive name is hotline_<version>_<os>_<arch>.tar.gz, with
# amd64 spelled x86_64. Binaries are built CGO_ENABLED=0 and are therefore
# statically linked -- there is deliberately no glibc/musl split to detect.
detect_platform() {
	uname_s="$(uname -s)"
	uname_m="$(uname -m)"

	case "$uname_s" in
	Linux) OS='linux' ;;
	Darwin) OS='darwin' ;;
	*) unsupported "$uname_s" "$uname_m" ;;
	esac

	case "$uname_m" in
	x86_64 | amd64) ARCH='x86_64' ;;
	arm64 | aarch64) ARCH='arm64' ;;
	*) unsupported "$uname_s" "$uname_m" ;;
	esac
}

unsupported() {
	die "no prebuilt binary for $1/$2.

hotline ships linux and macOS on x86_64 and arm64. For anything else, build
from source (Go 1.26+):

    go install github.com/${REPO}/cmd/hotline@latest

Every published asset is listed at ${DEFAULT_RELEASES}"
}

require() {
	command -v "$1" >/dev/null 2>&1 || die "$1 is required but not installed."
}

sha256_of() {
	if command -v sha256sum >/dev/null 2>&1; then
		sha256sum "$1" | awk '{print $1}'
	elif command -v shasum >/dev/null 2>&1; then
		shasum -a 256 "$1" | awk '{print $1}'
	elif command -v openssl >/dev/null 2>&1; then
		openssl dgst -sha256 "$1" | awk '{print $NF}'
	else
		die 'need one of sha256sum, shasum, or openssl to verify the download.'
	fi
}

# Resolve "latest" from the releases/latest redirect rather than the GitHub API:
# no rate limit, no token, no JSON parser.
resolve_tag() {
	if [ -n "${HOTLINE_VERSION:-}" ]; then
		case "$HOTLINE_VERSION" in
		v*) printf '%s' "$HOTLINE_VERSION" ;;
		*) printf 'v%s' "$HOTLINE_VERSION" ;;
		esac
		return
	fi

	resolved="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "${RELEASES}/latest" 2>/dev/null || true)"
	resolved="${resolved##*/}"
	case "$resolved" in
	v[0-9]*) printf '%s' "$resolved" ;;
	*) die "could not work out the latest release. Pin one instead:

    curl -fsSL https://hotline.dev/install.sh | HOTLINE_VERSION=v0.11.0 sh

Releases: ${DEFAULT_RELEASES}" ;;
	esac
}

fetch() {
	curl -fsSL "$1" -o "$2" || die "download failed: $1"
}

fetch_optional() {
	curl -fsSL "$1" -o "$2" 2>/dev/null || return 1
}

verify_signature() {
	sums="$1"

	if ! fetch_optional "${BASE}/checksums.txt.sig" "${sums}.sig" ||
		! fetch_optional "${BASE}/checksums.txt.pem" "${sums}.pem"; then
		warn_loudly \
			"Release ${TAG} publishes no signature for checksums.txt." \
			"The checksum below still proves the download is intact, but nothing" \
			"proves the checksums file itself came from the hotline release workflow." \
			"Releases from v0.11.0 onward are signed; older ones are not."
		return
	fi

	if ! command -v cosign >/dev/null 2>&1; then
		warn_loudly \
			"cosign is not installed, so the signature on checksums.txt was NOT verified." \
			"The SHA-256 check below still runs and still has to pass." \
			"What you lose is the proof that checksums.txt came from us." \
			"To get it: install cosign (https://docs.sigstore.dev/cosign/installation)" \
			"and re-run this installer."
		return
	fi

	cosign verify-blob \
		--certificate "${sums}.pem" \
		--signature "${sums}.sig" \
		--certificate-identity-regexp "$COSIGN_IDENTITY" \
		--certificate-oidc-issuer "$COSIGN_ISSUER" \
		"$sums" >/dev/null 2>&1 ||
		die "SIGNATURE VERIFICATION FAILED for checksums.txt of ${TAG}.

Nothing was installed. Do not run anything you downloaded from this release
until you know why. Report it: https://github.com/${REPO}/security"

	say 'signature ok    (cosign, keyless, release.yml)'
}

verify_checksum() {
	archive="$1"
	sums="$2"

	want="$(awk -v f="$ARCHIVE_NAME" '$2 == f {print $1}' "$sums" | head -n 1)"
	[ -n "$want" ] || die "${ARCHIVE_NAME} is not listed in checksums.txt for ${TAG}."

	got="$(sha256_of "$archive")"
	[ "$want" = "$got" ] || die "CHECKSUM MISMATCH for ${ARCHIVE_NAME}.

  expected  ${want}
  got       ${got}

Nothing was installed. The download was corrupted or tampered with; retry, and
if it happens twice report it at https://github.com/${REPO}/security"

	say "checksum ok     ${got}"
}

path_hint() {
	dir="$1"
	case ":${PATH}:" in
	*":${dir}:"*) return ;;
	esac

	shell="${SHELL:-/bin/sh}"
	case "${shell##*/}" in
	zsh) rc="$HOME/.zshrc" line="export PATH=\"${dir}:\$PATH\"" ;;
	bash) rc="$HOME/.bashrc" line="export PATH=\"${dir}:\$PATH\"" ;;
	fish) rc="$HOME/.config/fish/config.fish" line="fish_add_path ${dir}" ;;
	*) rc="$HOME/.profile" line="export PATH=\"${dir}:\$PATH\"" ;;
	esac

	say ''
	say "${dir} is not on your PATH. Add it:"
	say ''
	say "    echo '${line}' >> ${rc}"
	say ''
	say "Then open a new shell, or run: ${line}"
}

main() {
	require curl
	require tar
	detect_platform

	if [ "$RELEASES" != "$DEFAULT_RELEASES" ]; then
		warn_loudly "Installing from ${RELEASES}, not from GitHub Releases."
	fi

	TAG="$(resolve_tag)"
	VERSION="${TAG#v}"
	BASE="${RELEASES}/download/${TAG}"
	ARCHIVE_NAME="hotline_${VERSION}_${OS}_${ARCH}.tar.gz"
	DEST_DIR="${HOTLINE_INSTALL_DIR:-$HOME/.local/bin}"

	say "hotline ${TAG}  ${OS}/${ARCH}  ->  ${DEST_DIR}"

	TMP="$(mktemp -d "${TMPDIR:-/tmp}/hotline-install.XXXXXX")" ||
		die 'could not create a temporary directory.'
	trap cleanup EXIT INT TERM

	fetch "${BASE}/${ARCHIVE_NAME}" "${TMP}/${ARCHIVE_NAME}"
	fetch "${BASE}/checksums.txt" "${TMP}/checksums.txt"

	verify_signature "${TMP}/checksums.txt"
	verify_checksum "${TMP}/${ARCHIVE_NAME}" "${TMP}/checksums.txt"

	tar -xzf "${TMP}/${ARCHIVE_NAME}" -C "$TMP" hotline ||
		die "${ARCHIVE_NAME} does not contain a hotline binary."

	mkdir -p "$DEST_DIR" || die "cannot create ${DEST_DIR}."
	[ -w "$DEST_DIR" ] || die "${DEST_DIR} is not writable.

Pick somewhere you own:

    curl -fsSL https://hotline.dev/install.sh | HOTLINE_INSTALL_DIR=\$HOME/bin sh"

	# Stage inside the destination so the final step is an atomic rename on the
	# same filesystem. That is what makes a re-run an in-place upgrade: an old
	# hotline can be running and the swap still succeeds, and a failure anywhere
	# above leaves the previous binary untouched.
	staged="${DEST_DIR}/.hotline.install.$$"
	cp "${TMP}/hotline" "$staged" || die "could not write to ${DEST_DIR}."
	chmod 0755 "$staged"
	mv -f "$staged" "${DEST_DIR}/hotline" || {
		rm -f "$staged"
		die "could not install into ${DEST_DIR}."
	}

	"${DEST_DIR}/hotline" --version >/dev/null 2>&1 ||
		die "${DEST_DIR}/hotline was installed but will not run on this machine."

	say "installed       $("${DEST_DIR}/hotline" --version)"
	path_hint "$DEST_DIR"
	say ''
	say 'Next:  hotline setup     https://hotline.dev/docs/'
}

# Everything above is a definition; this is the only line that runs. A truncated
# download therefore does nothing at all instead of half-installing.
main "$@"
