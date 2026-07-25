package app

import (
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"mime"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"sync"
	"time"
)

const (
	blobChunkBytes = 256 << 10
	blobMaxBytes   = 50 << 20
	blobExpiry     = 120 * time.Second
)

type blobRecord struct {
	Xfer string `json:"xfer"`
	Mime string `json:"mime"`
	Size int64  `json:"size"`
	Path string `json:"path"`
}

type inboundBlob struct {
	mime    string
	size    int64
	chunks  int
	parts   map[int][]byte
	started time.Time
}

type blobRegistry struct {
	mu        sync.Mutex
	stateDir  string
	path      string
	outbound  map[string]blobRecord
	completed map[string]blobRecord
	inbound   map[string]*inboundBlob
	clock     func() time.Time
}

func newBlobRegistry(stateDir string) *blobRegistry {
	b := &blobRegistry{stateDir: stateDir, path: filepath.Join(stateDir, "blobs.json"), outbound: map[string]blobRecord{}, completed: map[string]blobRecord{}, inbound: map[string]*inboundBlob{}, clock: time.Now}
	data, err := os.ReadFile(b.path)
	if err == nil {
		var rows []blobRecord
		if json.Unmarshal(data, &rows) == nil {
			for _, row := range rows {
				b.outbound[row.Xfer] = row
				// Persisted inbound uploads remain downloadable by the harness and
				// replayable to sibling devices after a box restart.
				b.completed[row.Xfer] = row
			}
		}
	}
	return b
}

func (b *blobRegistry) register(path, mediaType string) (blobRecord, error) {
	f, err := os.Open(path)
	if err != nil {
		return blobRecord{}, err
	}
	defer f.Close()
	h := sha256.New()
	n, err := io.Copy(h, io.LimitReader(f, blobMaxBytes+1))
	if err != nil {
		return blobRecord{}, err
	}
	if n <= 0 || n > blobMaxBytes {
		return blobRecord{}, fmt.Errorf("blob size %d outside supported range", n)
	}
	xfer := "x-" + base64.RawURLEncoding.EncodeToString(h.Sum(nil)[:18])
	rec := blobRecord{Xfer: xfer, Mime: mediaType, Size: n, Path: path}
	b.mu.Lock()
	b.outbound[xfer] = rec
	err = b.saveLocked()
	b.mu.Unlock()
	return rec, err
}

func (b *blobRegistry) frames(xfer string) ([][]byte, error) {
	b.mu.Lock()
	rec, ok := b.outbound[xfer]
	b.mu.Unlock()
	if !ok {
		return nil, errors.New("unknown blob transfer")
	}
	data, err := os.ReadFile(rec.Path)
	if err != nil {
		return nil, err
	}
	chunks := (len(data) + blobChunkBytes - 1) / blobChunkBytes
	out := [][]byte{mustMarshal(map[string]any{"t": "blob_begin", "xfer": rec.Xfer, "mime": rec.Mime, "size": len(data), "chunks": chunks})}
	for i := 0; i < chunks; i++ {
		lo := i * blobChunkBytes
		hi := lo + blobChunkBytes
		if hi > len(data) {
			hi = len(data)
		}
		out = append(out, mustMarshal(map[string]any{"t": "blob_chunk", "xfer": rec.Xfer, "i": i, "data": base64.StdEncoding.EncodeToString(data[lo:hi])}))
	}
	out = append(out, mustMarshal(map[string]any{"t": "blob_end", "xfer": rec.Xfer}))
	return out, nil
}

func (b *blobRegistry) begin(xfer, mediaType string, size int64, chunks int) error {
	if len(xfer) < 4 || len(xfer) > 64 || mediaType == "" || len(mediaType) > 128 ||
		strings.ContainsAny(mediaType, "\x00\r\n") || size <= 0 || size > blobMaxBytes || chunks < 1 || chunks > 200 {
		return errors.New("invalid blob_begin")
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	b.expireLocked()
	b.inbound[xfer] = &inboundBlob{mime: mediaType, size: size, chunks: chunks, parts: map[int][]byte{}, started: b.clock()}
	return nil
}

func (b *blobRegistry) chunk(xfer string, i int, encoded string) error {
	data, err := base64.StdEncoding.DecodeString(encoded)
	if err != nil || len(data) > blobChunkBytes {
		return errors.New("invalid blob_chunk data")
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	b.expireLocked()
	in := b.inbound[xfer]
	if in == nil || i < 0 || i >= in.chunks {
		return errors.New("unknown blob transfer or chunk index")
	}
	in.parts[i] = data
	return nil
}

func (b *blobRegistry) end(xfer string) (blobRecord, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	b.expireLocked()
	in := b.inbound[xfer]
	if in == nil || len(in.parts) != in.chunks {
		return blobRecord{}, errors.New("incomplete blob transfer")
	}
	var data []byte
	for i := 0; i < in.chunks; i++ {
		data = append(data, in.parts[i]...)
	}
	if int64(len(data)) != in.size {
		return blobRecord{}, errors.New("blob size mismatch")
	}
	dir := filepath.Join(b.stateDir, "attachments")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return blobRecord{}, err
	}
	sum := sha256.Sum256(data)
	name := hex.EncodeToString(sum[:])[:16] + extensionForMime(in.mime)
	path := filepath.Join(dir, name)
	tmp, err := os.CreateTemp(dir, ".blob-*")
	if err != nil {
		return blobRecord{}, err
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName)
	_ = tmp.Chmod(0o600)
	if _, err := tmp.Write(data); err != nil {
		tmp.Close()
		return blobRecord{}, err
	}
	if err := tmp.Sync(); err != nil {
		tmp.Close()
		return blobRecord{}, err
	}
	if err := tmp.Close(); err != nil {
		return blobRecord{}, err
	}
	if err := os.Rename(tmpName, path); err != nil {
		return blobRecord{}, err
	}
	rec := blobRecord{Xfer: xfer, Mime: in.mime, Size: in.size, Path: path}
	b.completed[xfer] = rec
	b.outbound[xfer] = rec
	delete(b.inbound, xfer)
	if err := b.saveLocked(); err != nil {
		delete(b.completed, xfer)
		delete(b.outbound, xfer)
		return blobRecord{}, err
	}
	return rec, nil
}

func (b *blobRegistry) resolve(xfer string) (blobRecord, bool) {
	b.mu.Lock()
	defer b.mu.Unlock()
	rec, ok := b.completed[xfer]
	return rec, ok
}

func (b *blobRegistry) expireLocked() {
	now := b.clock()
	for xfer, in := range b.inbound {
		if now.Sub(in.started) >= blobExpiry {
			delete(b.inbound, xfer)
		}
	}
}

func (b *blobRegistry) saveLocked() error {
	rows := make([]blobRecord, 0, len(b.outbound))
	for _, row := range b.outbound {
		rows = append(rows, row)
	}
	sort.Slice(rows, func(i, j int) bool { return rows[i].Xfer < rows[j].Xfer })
	data, err := json.MarshalIndent(rows, "", "  ")
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(b.path), 0o700); err != nil {
		return err
	}
	tmp := b.path + ".tmp"
	if err := os.WriteFile(tmp, append(data, '\n'), 0o600); err != nil {
		return err
	}
	if err := os.Rename(tmp, b.path); err != nil {
		_ = os.Remove(tmp)
		return err
	}
	return os.Chmod(b.path, 0o600)
}

var safeExtRE = regexp.MustCompile(`^\.[A-Za-z0-9]{1,8}$`)

func extensionForMime(mediaType string) string {
	ext, _ := mime.ExtensionsByType(mediaType)
	if len(ext) > 0 && safeExtRE.MatchString(ext[0]) {
		return strings.ToLower(ext[0])
	}
	return ""
}

func mimeFor(name string) string {
	if m := mime.TypeByExtension(filepath.Ext(name)); m != "" {
		return strings.Split(m, ";")[0]
	}
	return "application/octet-stream"
}
