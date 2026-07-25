package mc

import (
	"os"
	"path/filepath"
)

// ReadIndex returns the raw INDEX.md content (the operator/CLI view). Missing
// store ⇒ ("", false, nil); the caller decides whether to seed.
func (s *Store) ReadIndex() (string, bool, error) {
	raw, err := os.ReadFile(s.indexPath())
	if err != nil {
		if os.IsNotExist(err) {
			return "", false, nil
		}
		return "", false, err
	}
	return string(raw), true, nil
}

// ReadThread returns the raw markdown for a thread, looking first in threads/
// (live) then archive/ (closed). Unknown ⇒ ("", false, nil).
func (s *Store) ReadThread(slug string) (string, bool, error) {
	if !slugRe.MatchString(slug) {
		return "", false, nil
	}
	for _, dir := range []string{s.threadsDir(), s.archiveDir()} {
		raw, err := os.ReadFile(filepath.Join(dir, slug+".md"))
		if err == nil {
			return string(raw), true, nil
		}
		if !os.IsNotExist(err) {
			return "", false, err
		}
	}
	return "", false, nil
}

// ReadHandoff returns the raw handoff.md content. Missing ⇒ ("", false, nil).
func (s *Store) ReadHandoff() (string, bool, error) {
	raw, err := os.ReadFile(s.handoffPath())
	if err != nil {
		if os.IsNotExist(err) {
			return "", false, nil
		}
		return "", false, err
	}
	return string(raw), true, nil
}

// ListThreadSlugs returns the slugs of every live (non-archived) thread, sorted
// by updated descending — the operator's "what's open" list for `mission show`.
func (s *Store) ListThreadSlugs() ([]string, error) {
	ts, err := s.activeThreads()
	if err != nil {
		return nil, err
	}
	out := make([]string, 0, len(ts))
	for _, t := range ts {
		out = append(out, t.slug)
	}
	return out, nil
}
