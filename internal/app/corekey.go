package app

import (
	"crypto/ecdsa"
	"path/filepath"
)

// boxKeyFile is the box's persistent P-256 identity for the hotline-core control
// plane (core-v1 SPEC §3.1). It is a DISTINCT key from push-signing-key.json
// (the push-gateway credential) and is generated on the first core-mode start,
// never provisioned. The on-disk format and the load/generate/atomic-0600-write
// mechanics are shared verbatim with the push signing key (loadOrCreateSigningKey
// + atomicWriteFile) so there is exactly one JWK persistence path in the box.
const boxKeyFile = "box-key.json"

// loadOrCreateBoxKey loads <stateDir>/box-key.json or generates and atomically
// persists a fresh P-256 key (mode 0600) when the file is absent.
func loadOrCreateBoxKey(stateDir string) (*ecdsa.PrivateKey, error) {
	return loadOrCreateSigningKey(filepath.Join(stateDir, boxKeyFile))
}

// publicJWKFor returns the {kty,crv,x,y} public JWK for a P-256 key, the shape
// the core binds to a room on register (SPEC §3.2). x/y are base64url (no
// padding) of the 32-byte big-endian coordinates.
func publicJWKFor(priv *ecdsa.PrivateKey) map[string]string {
	return map[string]string{
		"kty": "EC", "crv": "P-256",
		"x": scalarBase64URL(priv.PublicKey.X),
		"y": scalarBase64URL(priv.PublicKey.Y),
	}
}
