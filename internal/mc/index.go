package mc

import (
	"os"
	"strings"
	"time"
)

// regenIndex rewrites the Threads table between the mc:threads markers from the
// current thread front-matter, preserving everything outside the markers
// byte-for-byte. A missing INDEX.md is seeded with the header+markers first; a
// file with no markers gets them appended, so the operator can never mangle the
// file into a state the tool can't repair.
func (s *Store) regenIndex() error {
	threads, err := s.activeThreads()
	if err != nil {
		return err
	}

	var table strings.Builder
	table.WriteString("| Thread | Status | Summary → next | Updated |\n")
	table.WriteString("|---|---|---|---|\n")
	for _, t := range threads {
		summaryNext := orDash(t.summary)
		if t.next != "" {
			summaryNext = orDash(t.summary) + " → " + t.next
		}
		table.WriteString("| " + t.slug + " | " + orValue(t.status, "active") + " | " +
			cellEscape(summaryNext) + " | " + dateOf(t.updated) + " |\n")
	}
	block := threadsBeginMarker + "\n" + table.String() + threadsEndMarker

	raw, err := os.ReadFile(s.indexPath())
	if err != nil {
		if !os.IsNotExist(err) {
			return err
		}
		raw = []byte("# INDEX — mission control map\n\n" + standingHeader + "\n\n")
	}
	out := replaceMarkerBlock(string(raw), block)
	return atomicWrite(s.indexPath(), []byte(out))
}

// replaceMarkerBlock swaps the region from threadsBeginMarker to threadsEndMarker
// (inclusive) with block. If the markers are absent, block is appended after a
// blank line. Content outside the markers is untouched.
func replaceMarkerBlock(doc, block string) string {
	bi := strings.Index(doc, threadsBeginMarker)
	ei := strings.Index(doc, threadsEndMarker)
	if bi < 0 || ei < 0 || ei < bi {
		if !strings.HasSuffix(doc, "\n") {
			doc += "\n"
		}
		return doc + "\n" + block + "\n"
	}
	ei += len(threadsEndMarker)
	return doc[:bi] + block + doc[ei:]
}

// addStandingNote appends "- YYYY-MM-DD: text" to the standing-notes list under
// the "## Standing notes" header, preserving the rest of INDEX.md. On overflow
// (> maxStandingNotes) the oldest notes roll into archive/standing-YYYY-MM.md.
func (s *Store) addStandingNote(text string) error {
	raw, err := os.ReadFile(s.indexPath())
	if err != nil {
		if !os.IsNotExist(err) {
			return err
		}
		raw = []byte("# INDEX — mission control map\n\n" + standingHeader + "\n\n" +
			threadsBeginMarker + "\n" + threadsEndMarker + "\n")
	}
	lines := strings.Split(string(raw), "\n")
	hi := -1
	for i, l := range lines {
		if strings.TrimRight(l, " \t") == standingHeader {
			hi = i
			break
		}
	}
	if hi < 0 {
		// No standing-notes section — insert one just before the threads marker
		// (or at the end), so subsequent notes have a home.
		insertAt := len(lines)
		for i, l := range lines {
			if strings.TrimSpace(l) == threadsBeginMarker {
				insertAt = i
				break
			}
		}
		section := []string{standingHeader, ""}
		lines = append(lines[:insertAt], append(section, lines[insertAt:]...)...)
		hi = insertAt
	}

	// The standing-notes region runs from just after the header to the next
	// section boundary (a "## " header or the threads marker) or EOF. The tool
	// owns only the bullet lines inside it — blank lines and operator prose are
	// preserved verbatim (write-ownership rule: outside-marker content is the
	// operator's). So we splice the new bullet in among the existing ones and
	// remove overflow bullets in place, never rewriting the whole section.
	end := len(lines)
	for i := hi + 1; i < len(lines); i++ {
		l := strings.TrimSpace(lines[i])
		if strings.HasPrefix(lines[i], "## ") || l == threadsBeginMarker {
			end = i
			break
		}
	}
	section := append([]string{}, lines[hi+1:end]...)

	isBullet := func(l string) bool { return strings.HasPrefix(strings.TrimSpace(l), "- ") }
	bulletIdxs := func() []int {
		var idxs []int
		for i, l := range section {
			if isBullet(l) {
				idxs = append(idxs, i)
			}
		}
		return idxs
	}

	// Insert the new bullet right after the last existing bullet, or — when the
	// section has no bullets yet — at the section head, so prose below it stays put.
	newBullet := "- " + s.now().UTC().Format("2006-01-02") + ": " + oneLine(text)
	if idxs := bulletIdxs(); len(idxs) == 0 {
		section = append([]string{newBullet}, section...)
	} else {
		at := idxs[len(idxs)-1] + 1
		section = append(section[:at], append([]string{newBullet}, section[at:]...)...)
	}

	// Overflow: roll the oldest bullets (earliest in file order) into the archive
	// and delete just those lines, leaving all prose/blank lines intact.
	if idxs := bulletIdxs(); len(idxs) > maxStandingNotes {
		overflowIdxs := idxs[:len(idxs)-maxStandingNotes]
		overflow := make([]string, 0, len(overflowIdxs))
		for _, i := range overflowIdxs {
			overflow = append(overflow, strings.TrimSpace(section[i]))
		}
		if err := s.rollStandingNotes(overflow); err != nil {
			return err
		}
		for k := len(overflowIdxs) - 1; k >= 0; k-- {
			i := overflowIdxs[k]
			section = append(section[:i], section[i+1:]...)
		}
	}

	rebuilt := append([]string{}, lines[:hi+1]...)
	rebuilt = append(rebuilt, section...)
	rebuilt = append(rebuilt, lines[end:]...)
	return atomicWrite(s.indexPath(), []byte(strings.Join(rebuilt, "\n")))
}

// rollStandingNotes appends pruned notes to archive/standing-YYYY-MM.md, grouped
// by the current month.
func (s *Store) rollStandingNotes(notes []string) error {
	if len(notes) == 0 {
		return nil
	}
	if err := os.MkdirAll(s.archiveDir(), 0o700); err != nil {
		return err
	}
	name := "standing-" + s.now().UTC().Format("2006-01") + ".md"
	path := s.archiveDir() + string(os.PathSeparator) + name
	var b strings.Builder
	if prev, err := os.ReadFile(path); err == nil {
		b.Write(prev)
		if len(prev) > 0 && !strings.HasSuffix(string(prev), "\n") {
			b.WriteString("\n")
		}
	} else if !os.IsNotExist(err) {
		return err
	}
	for _, n := range notes {
		b.WriteString(n + "\n")
	}
	return atomicWrite(path, []byte(b.String()))
}

// standingNotes returns the standing-notes bullets (text after the date prefix
// preserved as written), in file order (oldest first).
func (s *Store) standingNotes() []string {
	raw, err := os.ReadFile(s.indexPath())
	if err != nil {
		return nil
	}
	lines := strings.Split(string(raw), "\n")
	hi := -1
	for i, l := range lines {
		if strings.TrimRight(l, " \t") == standingHeader {
			hi = i
			break
		}
	}
	if hi < 0 {
		return nil
	}
	var out []string
	for i := hi + 1; i < len(lines); i++ {
		l := strings.TrimSpace(lines[i])
		if strings.HasPrefix(lines[i], "## ") || l == threadsBeginMarker {
			break
		}
		if strings.HasPrefix(l, "- ") {
			out = append(out, strings.TrimPrefix(l, "- "))
		}
	}
	return out
}

// cellEscape neutralizes the pipe and newline so a summary/next never breaks the
// markdown table row.
func cellEscape(s string) string {
	s = strings.ReplaceAll(s, "\n", " ")
	s = strings.ReplaceAll(s, "|", "\\|")
	return s
}

// dateOf renders the date portion of an RFC3339 timestamp (or passes through a
// value that is already a bare date / unparseable).
func dateOf(ts string) string {
	if ts == "" {
		return ""
	}
	if t, err := time.Parse(time.RFC3339, ts); err == nil {
		return t.UTC().Format("2006-01-02")
	}
	if len(ts) >= 10 {
		return ts[:10]
	}
	return ts
}
