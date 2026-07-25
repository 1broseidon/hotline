package app

import (
	"bufio"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
)

const defaultCIDDedupCap = 1024

type cidDeduper struct {
	cap     int
	entries map[string]struct{}
	order   []string
	path    string
}

func newPersistentCIDDeduper(capacity int, path string) *cidDeduper {
	if capacity < defaultCIDDedupCap {
		capacity = defaultCIDDedupCap
	}
	d := &cidDeduper{cap: capacity, entries: map[string]struct{}{}, path: path}
	f, err := os.Open(path)
	if err == nil {
		s := bufio.NewScanner(f)
		for s.Scan() {
			var row struct {
				Key string `json:"key"`
			}
			if json.Unmarshal(s.Bytes(), &row) == nil {
				d.addMemory(row.Key)
			}
		}
		_ = f.Close()
	}
	return d
}

func deliveryKey(deviceID, cid string) string {
	sum := sha256.Sum256([]byte(deviceID + "\x00" + cid))
	return hex.EncodeToString(sum[:])
}

func (d *cidDeduper) seen(key string) bool {
	_, ok := d.entries[key]
	return ok
}

func (d *cidDeduper) add(key string) {
	if key == "" || d.seen(key) {
		return
	}
	d.addMemory(key)
	if err := os.MkdirAll(filepath.Dir(d.path), 0o700); err != nil {
		return
	}
	f, err := os.OpenFile(d.path, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o600)
	if err != nil {
		return
	}
	row, _ := json.Marshal(map[string]string{"key": key})
	if _, err := fmt.Fprintf(f, "%s\n", row); err == nil {
		_ = f.Sync()
	}
	_ = f.Close()
}

func (d *cidDeduper) addMemory(key string) {
	if key == "" || d.seen(key) {
		return
	}
	if len(d.order) >= d.cap {
		delete(d.entries, d.order[0])
		d.order = d.order[1:]
	}
	d.order = append(d.order, key)
	d.entries[key] = struct{}{}
}

func (d *cidDeduper) close() {}
