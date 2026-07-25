package app

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"os"
	"path/filepath"
)

const maxUploadBytes = 50 << 20

func (s *Server) attDir() string { return filepath.Join(s.cfg.StateDir, "attachments") }

func (s *Server) importAttachment(path string) (*fileRef, error) {
	info, err := os.Stat(path)
	if err != nil {
		return nil, err
	}
	if !info.Mode().IsRegular() || info.Size() <= 0 || info.Size() > maxUploadBytes {
		return nil, fmt.Errorf("file size %d outside supported range", info.Size())
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	sum := sha256.Sum256(data)
	name := hex.EncodeToString(sum[:])[:16] + filepath.Ext(path)
	if err := os.MkdirAll(s.attDir(), 0o700); err != nil {
		return nil, err
	}
	dst := filepath.Join(s.attDir(), name)
	if err := os.WriteFile(dst+".tmp", data, 0o600); err != nil {
		return nil, err
	}
	if err := os.Rename(dst+".tmp", dst); err != nil {
		_ = os.Remove(dst + ".tmp")
		return nil, err
	}
	mediaType := mimeFor(name)
	blob, err := s.blobs.register(dst, mediaType)
	if err != nil {
		return nil, err
	}
	return &fileRef{ID: name, Name: filepath.Base(path), Mime: mediaType, Size: info.Size(), Xfer: blob.Xfer}, nil
}
