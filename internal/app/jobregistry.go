package app

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
)

const (
	jobRegistrySchemaVersion = 1
	jobRegistryFile          = "registry.json"
)

// jobRegistryDisk and jobRegistryDiskRecord are private app-owned disk types.
// The in-memory registry stays free to evolve independently of this schema.
type jobRegistryDisk struct {
	Version  int                     `json:"version"`
	Sequence int                     `json:"sequence"`
	Active   []jobRegistryDiskRecord `json:"active"`
}

type jobRegistryDiskRecord struct {
	JobID     string   `json:"jobID"`
	ElementID string   `json:"elementID"`
	MessageID string   `json:"messageID"`
	ChatID    string   `json:"chatID"`
	Title     string   `json:"title"`
	Detail    string   `json:"detail"`
	Progress  *float64 `json:"progress"`
	StartedAt int64    `json:"startedAt"`
	Stale     bool     `json:"stale"`
}

func jobRegistryStoragePath(stateDir string) string {
	return filepath.Join(stateDir, "cards", jobRegistryFile)
}

// newPersistentJobRegistry loads the app-owned card registry at path. A missing
// file is materialized immediately so its directory and permissions are in
// place before the first card. Any returned error is non-fatal to callers: the
// returned registry remains usable in memory and keeps trying to persist future
// mutations.
func newPersistentJobRegistry(path string) (*jobRegistry, error) {
	r := newJobRegistry()
	r.storagePath = path

	raw, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			r.mu.Lock()
			err = r.persistLocked()
			r.mu.Unlock()
			return r, err
		}
		return r, fmt.Errorf("read %s: %w", path, err)
	}

	var disk jobRegistryDisk
	if err := json.Unmarshal(raw, &disk); err != nil {
		return r, fmt.Errorf("decode %s: %w", path, err)
	}
	if disk.Version != jobRegistrySchemaVersion {
		return r, fmt.Errorf("decode %s: unsupported version %d", path, disk.Version)
	}

	r.seq = disk.Sequence
	for _, row := range disk.Active {
		// Always advance past every record id, even if a malformed row cannot be
		// restored as an actionable card.
		if n := jobSeqOf(row.JobID); n > r.seq {
			r.seq = n
		}
		if row.JobID == "" || row.ElementID == "" || row.MessageID == "" || row.Title == "" {
			continue
		}
		progress := cloneJobProgress(row.Progress)
		r.jobs[row.JobID] = &jobRecord{
			jobID: row.JobID, elementID: row.ElementID, messageID: row.MessageID,
			chatID: row.ChatID, title: row.Title, detail: row.Detail,
			progress: progress, startedAt: row.StartedAt, stale: row.Stale,
		}
	}
	if err := os.Chmod(filepath.Dir(path), 0o700); err != nil {
		return r, fmt.Errorf("chmod %s: %w", filepath.Dir(path), err)
	}
	if err := os.Chmod(path, 0o600); err != nil {
		return r, fmt.Errorf("chmod %s: %w", path, err)
	}
	return r, nil
}

// persistLocked atomically replaces the registry snapshot. Caller holds r.mu.
// A memory-only registry has no storagePath and treats persistence as a no-op.
func (r *jobRegistry) persistLocked() error {
	if r.storagePath == "" {
		return nil
	}

	ids := make([]string, 0, len(r.jobs))
	for id := range r.jobs {
		ids = append(ids, id)
		if n := jobSeqOf(id); n > r.seq {
			r.seq = n
		}
	}
	sort.Strings(ids)
	disk := jobRegistryDisk{
		Version:  jobRegistrySchemaVersion,
		Sequence: r.seq,
		Active:   make([]jobRegistryDiskRecord, 0, len(ids)),
	}
	for _, id := range ids {
		rec := r.jobs[id]
		disk.Active = append(disk.Active, jobRegistryDiskRecord{
			JobID: rec.jobID, ElementID: rec.elementID, MessageID: rec.messageID,
			ChatID: rec.chatID, Title: rec.title, Detail: rec.detail,
			Progress: cloneJobProgress(rec.progress), StartedAt: rec.startedAt, Stale: rec.stale,
		})
	}
	return atomicWriteJobRegistry(r.storagePath, disk)
}

func atomicWriteJobRegistry(path string, disk jobRegistryDisk) error {
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return fmt.Errorf("mkdir %s: %w", dir, err)
	}
	if err := os.Chmod(dir, 0o700); err != nil {
		return fmt.Errorf("chmod %s: %w", dir, err)
	}
	data, err := json.MarshalIndent(disk, "", "  ")
	if err != nil {
		return fmt.Errorf("encode registry: %w", err)
	}
	data = append(data, '\n')

	tmp := path + ".tmp"
	f, err := os.OpenFile(tmp, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, 0o600)
	if err != nil {
		return fmt.Errorf("open temp registry: %w", err)
	}
	removeTemp := true
	defer func() {
		if removeTemp {
			_ = os.Remove(tmp)
		}
	}()
	if err := f.Chmod(0o600); err != nil {
		_ = f.Close()
		return fmt.Errorf("chmod temp registry: %w", err)
	}
	if _, err := f.Write(data); err != nil {
		_ = f.Close()
		return fmt.Errorf("write temp registry: %w", err)
	}
	if err := f.Sync(); err != nil {
		_ = f.Close()
		return fmt.Errorf("sync temp registry: %w", err)
	}
	if err := f.Close(); err != nil {
		return fmt.Errorf("close temp registry: %w", err)
	}
	if err := os.Rename(tmp, path); err != nil {
		return fmt.Errorf("rename temp registry: %w", err)
	}
	removeTemp = false
	if err := os.Chmod(path, 0o600); err != nil {
		return fmt.Errorf("chmod registry: %w", err)
	}
	return nil
}

func cloneJobProgress(progress *float64) *float64 {
	if progress == nil {
		return nil
	}
	value := *progress
	return &value
}

func logJobRegistryFailure(operation string, err error) {
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: app card registry %s failed: %v\n", operation, err)
	}
}
