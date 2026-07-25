package app

import (
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"
)

const (
	relayStateFile               = "relay-state.json"
	maxLiveActivitiesPerDevice   = 32
	liveActivityRegisteredAtTime = time.RFC3339Nano
)

var (
	roomIDRE       = regexp.MustCompile(`^[A-Za-z0-9_-]{22}$`)
	deviceIDRE     = regexp.MustCompile(`^dev-[0-9a-f]{6,32}$`)
	deviceLookupRE = regexp.MustCompile(`^dev-[A-Za-z0-9_-]{6,32}$`)
)

type DeviceState string

const (
	DeviceActive  DeviceState = "active"
	DeviceUnbound DeviceState = "unbound"
	DeviceBanned  DeviceState = "banned"

	// DeviceRevoked is the legacy non-terminal state written by link rotation
	// before explicit operator bans had their own durable state.
	DeviceRevoked DeviceState = "revoked"
)

// RoomState is the multi-device room lifecycle (SPEC §1). Only the terminal
// "dead" value is ever persisted; open/bound are computed from an absent state
// (a room with a live device is bound, otherwise open). Persisting only "dead"
// keeps additive mints and --rotate-all byte-for-byte compatible with the
// pre-multi-device on-disk shape and makes the load-time migration implicit.
type RoomState string

const (
	RoomOpen  RoomState = "open"
	RoomBound RoomState = "bound"
	RoomDead  RoomState = "dead"
)

const (
	// maxBoundRooms caps the number of concurrently served (open|bound) rooms.
	// Overridable via HOTLINE_MAX_ROOMS (clamped 1..16).
	maxBoundRooms = 8
	// openRoomExpiry retires an open room that never linked a device (SPEC §1).
	openRoomExpiry = 48 * time.Hour
)

type RoomRecord struct {
	ID         string `json:"id"`
	URL        string `json:"url"`
	Name       string `json:"name"`
	SecretHash string `json:"secret_hash"`
	CreatedAt  string `json:"created_at"`
	// State is the multi-device room lifecycle marker. Empty means an active
	// (open|bound) room — only a "dead" room is persisted explicitly (see
	// RoomState). Old relay-state files have no state and load as active.
	State RoomState `json:"state,omitempty"`
	// Envelope marks an e1 E2E pairing (core-v1 SPEC §1): the connector wraps
	// every v2 frame in the e1 envelope for this room's lifetime. Empty/false for
	// every plaintext room (the default, and every room minted outside core mode).
	Envelope bool `json:"envelope,omitempty"`
	// Secret is the raw pairing secret, persisted ONLY for envelope rooms so the
	// box can derive the e1 content keys and the register auth_hash. Plaintext
	// rooms keep only SecretHash (the secret never lands on disk), so a box that
	// never enters core mode has byte-for-byte the same relay-state.json as before.
	Secret string `json:"secret,omitempty"`
}

type LiveActivityRegistration struct {
	Token        string `json:"token"`
	RegisteredAt string `json:"registered_at"`
}

type DeviceRecord struct {
	ID           string `json:"id"`
	Room         string `json:"room"`
	SecretHash   string `json:"secret_hash"`
	PushToken    string `json:"push_token,omitempty"`
	PushPlatform string `json:"push_platform,omitempty"`
	// PushKeyID is the gateway credential id returned by /registrations/complete
	// for the current PushToken (gateway mode only; empty for the Expo path).
	// PushRegState tracks the registration lifecycle: "" (none/needs registration),
	// "pending", "active", or "dropped".
	PushKeyID    string `json:"push_key_id,omitempty"`
	PushRegState string `json:"push_reg_state,omitempty"`
	// PushPreviewClear is the device's own push-preview preference (FB23): true =
	// this device wants the full message text in its push body, false = generic
	// "New Message". A nil pointer means the device never expressed a preference,
	// so the wake path falls back to the box env default (HOTLINE_PUSH_PREVIEW).
	// omitempty keeps a device that never toggled byte-identical to the old shape.
	PushPreviewClear *bool `json:"push_preview_clear,omitempty"`
	// JobCompletionPush is the device's FB44 successful-job notification
	// preference. nil defaults enabled for additive state compatibility; false
	// opts this device out and true explicitly opts it in.
	JobCompletionPush *bool `json:"job_completion_push,omitempty"`
	// LiveActivities holds direct APNs ActivityKit registrations by active job.
	// It is additive and omitted for devices that have never registered one.
	LiveActivities map[string]LiveActivityRegistration `json:"live_activities,omitempty"`
	State          DeviceState                         `json:"state"`
	LinkedAt       string                              `json:"linked_at"`
}

// LiveActivityTarget is an immutable store snapshot used by the asynchronous
// APNs sender. Token remains private transport data and must never be logged.
type LiveActivityTarget struct {
	DeviceID string
	JobID    string
	Token    string
}

type relayState struct {
	CurrentRoom string `json:"current_room,omitempty"`
	// Name is the box-owned assistant identity (FB21): one display name owned by
	// the box, not per-room. Seeded once (see SeedIdentityName) and thereafter the
	// single source of truth for the bot's name, streamed live to every device via
	// the agent_state snapshot. Empty on pre-FB21 state files; they seed lazily on
	// the next boot. omitempty keeps a never-seeded box byte-identical to the old
	// on-disk shape.
	Name    string                  `json:"name,omitempty"`
	Rooms   map[string]RoomRecord   `json:"rooms"`
	Devices map[string]DeviceRecord `json:"devices"`
}

type RelayStore struct {
	mu   sync.Mutex
	path string
	st   relayState
}

type Link struct {
	URL      string
	Room     string
	Secret   string
	Name     string
	URI      string
	Envelope bool
}

type VerifyResult int

const (
	VerifyUnauthorized VerifyResult = iota
	VerifyActive
	VerifyRevoked
)

func OpenRelayStore(stateDir string) (*RelayStore, error) {
	s := &RelayStore{path: filepath.Join(stateDir, relayStateFile)}
	s.st.Rooms = map[string]RoomRecord{}
	s.st.Devices = map[string]DeviceRecord{}
	data, err := os.ReadFile(s.path)
	if err != nil && !os.IsNotExist(err) {
		return nil, err
	}
	if err == nil {
		if err := json.Unmarshal(data, &s.st); err != nil {
			return nil, fmt.Errorf("decode relay state: %w", err)
		}
	}
	if s.st.Rooms == nil {
		s.st.Rooms = map[string]RoomRecord{}
	}
	if s.st.Devices == nil {
		s.st.Devices = map[string]DeviceRecord{}
	}
	return s, nil
}

func normalizeRendezvous(raw string) (string, error) {
	raw = strings.TrimRight(strings.TrimSpace(raw), "/")
	u, err := url.Parse(raw)
	if err != nil || u.Host == "" || (u.Scheme != "ws" && u.Scheme != "wss") {
		return "", fmt.Errorf("rendezvous URL must be ws:// or wss:// with a host")
	}
	if u.User != nil || u.RawQuery != "" || u.Fragment != "" {
		return "", fmt.Errorf("rendezvous URL must not contain userinfo, query, or fragment")
	}
	return raw, nil
}

func randomBase64URL(n int) (string, error) {
	b := make([]byte, n)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	return base64.RawURLEncoding.EncodeToString(b), nil
}

func secretHash(secret string) string {
	sum := sha256.Sum256([]byte(secret))
	return hex.EncodeToString(sum[:])
}

func PairingURI(base, room, secret, name string) string {
	return PairingURIMode(base, room, secret, name, false)
}

// PairingURIMode builds the pair URI, adding the additive core-v1 e=1 param when
// envelope is set (SPEC §1.2). e=1 marks the pairing envelope-mode; old parsers
// ignore the unknown param per v2 rules. The plaintext form (envelope=false) is
// byte-identical to the pre-core PairingURI output.
func PairingURIMode(base, room, secret, name string, envelope bool) string {
	return pairingURI(base, room, secret, name, envelope, "")
}

// FleetPairingURI builds a purpose-bound fleet pair URI (A2A v2 §2): the
// operator form plus the additive p=fleet param and a required e=1 envelope.
// `fleet join` requires p=fleet; the operator redeem path refuses it
// (PairParams.validateOperator). Old parsers ignore the unknown p param.
func FleetPairingURI(base, room, secret, name string) string {
	return pairingURI(base, room, secret, name, true, "fleet")
}

// pairingURI is the shared additive builder: purpose "" is the operator form,
// purpose "fleet" adds p=fleet.
func pairingURI(base, room, secret, name string, envelope bool, purpose string) string {
	q := url.Values{}
	q.Set("v", "1")
	q.Set("u", base)
	q.Set("r", room)
	q.Set("s", secret)
	q.Set("n", name)
	if envelope {
		q.Set("e", "1")
	}
	if purpose != "" {
		q.Set("p", purpose)
	}
	return "hotline://pair?" + q.Encode()
}

func (s *RelayStore) MintLink(base, name string) (Link, error) {
	return s.MintLinkMode(base, name, false)
}

// maxRooms reads the served-room cap from HOTLINE_MAX_ROOMS (clamped 1..16),
// defaulting to maxBoundRooms.
func maxRooms() int {
	n := maxBoundRooms
	if v := strings.TrimSpace(os.Getenv("HOTLINE_MAX_ROOMS")); v != "" {
		if parsed, err := strconv.Atoi(v); err == nil {
			n = parsed
		}
	}
	if n < 1 {
		n = 1
	}
	if n > 16 {
		n = 16
	}
	return n
}

// MaxRooms exposes the effective served-room cap for status/CLI reporting.
func MaxRooms() int { return maxRooms() }

// mintRoom builds a fresh room record + secret. Shared by the additive and
// rotate-all mint paths so the room construction stays byte-identical.
func mintRoom(base, name string, envelope bool) (RoomRecord, string, error) {
	base, err := normalizeRendezvous(base)
	if err != nil {
		return RoomRecord{}, "", err
	}
	if len([]rune(name)) > 64 {
		return RoomRecord{}, "", fmt.Errorf("assistant name exceeds 64 characters")
	}
	room, err := randomBase64URL(16)
	if err != nil {
		return RoomRecord{}, "", err
	}
	secret, err := randomBase64URL(32)
	if err != nil {
		return RoomRecord{}, "", err
	}
	r := RoomRecord{ID: room, URL: base, Name: name, SecretHash: secretHash(secret), CreatedAt: time.Now().UTC().Format(time.RFC3339), Envelope: envelope}
	if envelope {
		r.Secret = secret
	}
	return r, secret, nil
}

// MintLinkMode mints a new pairing ADDITIVELY (SPEC §2.1): the new room is
// inserted into the rooms map alongside every existing room and device — no
// prior pairing is touched. It fails at the served-room cap. current_room is
// still pointed at the newest room for old-binary rollback (SPEC §6), but new
// code never routes on it. envelope=false is byte-for-byte the legacy per-room
// shape (SecretHash only, no e param, no Secret on disk).
//
// The destructive whole-map rotation now lives in RotateAll.
func (s *RelayStore) MintLinkMode(base, name string, envelope bool) (Link, error) {
	r, secret, err := mintRoom(base, name, envelope)
	if err != nil {
		return Link{}, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	s.expireOpenRoomsLocked(time.Now())
	if s.servedRoomCountLocked() >= maxRooms() {
		return Link{}, fmt.Errorf("room cap reached (%d/%d): free a slot with `hotline relay revoke <device-id>` or replace all pairings with `hotline relay new-link --rotate-all`", s.servedRoomCountLocked(), maxRooms())
	}
	s.st.Rooms[r.ID] = r
	s.st.CurrentRoom = r.ID
	if err := s.saveLocked(); err != nil {
		return Link{}, err
	}
	return Link{URL: r.URL, Room: r.ID, Secret: secret, Name: name, URI: PairingURIMode(r.URL, r.ID, secret, name, envelope), Envelope: envelope}, nil
}

// RotateAll is the destructive panic-button mint (SPEC §2.1, `new-link
// --rotate-all`): it unbinds every non-banned device and REPLACES the whole
// rooms map with the single new room. This is byte-for-byte the pre-multi-device
// MintLinkMode behavior and the only path that mass-unbinds. It ignores the cap
// (it collapses to one room).
func (s *RelayStore) RotateAll(base, name string, envelope bool) (Link, error) {
	r, secret, err := mintRoom(base, name, envelope)
	if err != nil {
		return Link{}, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	for id, d := range s.st.Devices {
		if d.State != DeviceBanned {
			d.State = DeviceUnbound
		}
		d.LiveActivities = nil
		s.st.Devices[id] = d
	}
	s.st.Rooms = map[string]RoomRecord{r.ID: r}
	s.st.CurrentRoom = r.ID
	if err := s.saveLocked(); err != nil {
		return Link{}, err
	}
	return Link{URL: r.URL, Room: r.ID, Secret: secret, Name: name, URI: PairingURIMode(r.URL, r.ID, secret, name, envelope), Envelope: envelope}, nil
}

// IdentityName returns the box-owned assistant name (FB21) and whether it has
// been seeded yet. It reads fresh from disk so a rename done by another process
// (e.g. a device set_name handled by the running box while a CLI reads) is seen.
func (s *RelayStore) IdentityName() (string, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	return s.st.Name, strings.TrimSpace(s.st.Name) != ""
}

// SeedIdentityName installs the box identity ONCE (FB21 §1). If a name is already
// seeded it is left untouched and returned with seeded=false — the seed never
// re-rolls or overwrites on a later boot. Otherwise name becomes the durable
// identity, is persisted, and returned with seeded=true. The seed does NOT
// restamp existing room records (those stay as historical pre-connect
// placeholders); the live name reaches every device through the snapshot.
func (s *RelayStore) SeedIdentityName(name string) (stored string, seeded bool, err error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	if strings.TrimSpace(s.st.Name) != "" {
		return s.st.Name, false, nil
	}
	s.st.Name = name
	if err := s.saveLocked(); err != nil {
		return "", false, err
	}
	return name, true, nil
}

// SetIdentityName renames the box identity (FB21 §4, device set_name). It also
// restamps every non-dead room's Name to the new identity (FB21 §5) so
// `hotline relay status` and reconnect placeholders (welcomeFrame room name)
// agree with the live name. Callers validate the name first.
func (s *RelayStore) SetIdentityName(name string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	s.st.Name = name
	for id, r := range s.st.Rooms {
		if r.State == RoomDead {
			continue
		}
		r.Name = name
		s.st.Rooms[id] = r
	}
	return s.saveLocked()
}

// servedRoomCountLocked counts rooms that are not dead (open|bound) — the set
// the cap governs.
func (s *RelayStore) servedRoomCountLocked() int {
	n := 0
	for _, r := range s.st.Rooms {
		if r.State != RoomDead {
			n++
		}
	}
	return n
}

// roomHasDeviceLocked reports whether any device record points at the room —
// used to tell a never-linked "open" room from a room whose device unbound.
func (s *RelayStore) roomHasDeviceLocked(roomID string) bool {
	for _, d := range s.st.Devices {
		if d.Room == roomID {
			return true
		}
	}
	return false
}

// expireOpenRoomsLocked retires open rooms that never linked a device and are
// older than openRoomExpiry (SPEC §1 unclaimed-mint hygiene) by marking them
// dead, freeing the slot. Rooms with a device (bound or unbound-but-rebindable)
// are left alone; the operator frees those with `relay revoke`.
func (s *RelayStore) expireOpenRoomsLocked(now time.Time) {
	for id, r := range s.st.Rooms {
		if r.State == RoomDead || s.roomHasDeviceLocked(id) {
			continue
		}
		created, err := time.Parse(time.RFC3339, r.CreatedAt)
		if err != nil {
			continue
		}
		if now.Sub(created) >= openRoomExpiry {
			r.State = RoomDead
			s.st.Rooms[id] = r
		}
	}
}

// ServedRooms returns every room the connector should serve — every non-dead
// room — sorted by id for a deterministic spawn order (SPEC §3). It reloads the
// on-disk state so a `relay new-link` / `relay revoke` from a separate CLI
// process is observed within one poll.
func (s *RelayStore) ServedRooms() []RoomRecord {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	var out []RoomRecord
	for _, r := range s.st.Rooms {
		if r.State != RoomDead {
			out = append(out, r)
		}
	}
	sort.Slice(out, func(i, j int) bool { return out[i].ID < out[j].ID })
	return out
}

// RoomStateFor reports the display state of a room (SPEC §2.4): dead if
// tombstoned, bound if a live device rides it, else open.
func (s *RelayStore) RoomStateFor(r RoomRecord) RoomState {
	if r.State == RoomDead {
		return RoomDead
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	for _, d := range s.st.Devices {
		if d.Room == r.ID && d.State == DeviceActive {
			return RoomBound
		}
	}
	return RoomOpen
}

func (s *RelayStore) CurrentRoom() (RoomRecord, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	r, ok := s.st.Rooms[s.st.CurrentRoom]
	return r, ok
}

func (s *RelayStore) ActiveDevices() []DeviceRecord {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	var out []DeviceRecord
	for _, d := range s.st.Devices {
		if d.State == DeviceActive {
			out = append(out, d)
		}
	}
	sort.Slice(out, func(i, j int) bool { return out[i].ID < out[j].ID })
	return out
}

func (s *RelayStore) Devices() []DeviceRecord {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	out := make([]DeviceRecord, 0, len(s.st.Devices))
	for _, d := range s.st.Devices {
		out = append(out, d)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].ID < out[j].ID })
	return out
}

func hashEqual(gotSecret, encodedHash string) bool {
	got := sha256.Sum256([]byte(gotSecret))
	want, err := hex.DecodeString(encodedHash)
	if err != nil || len(want) != sha256.Size {
		var zero [sha256.Size]byte
		_ = subtle.ConstantTimeCompare(got[:], zero[:])
		return false
	}
	return subtle.ConstantTimeCompare(got[:], want) == 1
}

func (s *RelayStore) VerifyAndLink(room, deviceID, secret string) (VerifyResult, bool, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	if !deviceLookupRE.MatchString(deviceID) {
		return VerifyUnauthorized, false, nil
	}
	// A banned device is terminal regardless of room state — check it before the
	// room-liveness gate so a revoked device whose room was deaded still gets the
	// explicit "revoked" (4003) answer, not a generic unauthorized.
	d, exists := s.st.Devices[deviceID]
	if exists && d.State == DeviceBanned {
		return VerifyRevoked, false, nil
	}
	r, ok := s.st.Rooms[room]
	if !ok || r.State == RoomDead {
		return VerifyUnauthorized, false, nil
	}
	if !hashEqual(secret, r.SecretHash) {
		return VerifyUnauthorized, false, nil
	}
	if !exists && !deviceIDRE.MatchString(deviceID) {
		return VerifyUnauthorized, false, nil
	}
	for id, other := range s.st.Devices {
		if id != deviceID && other.Room == room && other.State == DeviceActive {
			return VerifyUnauthorized, false, nil
		}
	}
	if exists && d.Room == room && d.SecretHash == r.SecretHash && d.State == DeviceActive {
		return VerifyActive, false, nil
	}
	// A room change or rebind starts a new authenticated device lifecycle. Any
	// ActivityKit tokens belonged to the old binding and must not cross it.
	d.LiveActivities = nil
	d.ID = deviceID
	d.Room = room
	d.SecretHash = r.SecretHash
	d.State = DeviceActive
	d.LinkedAt = time.Now().UTC().Format(time.RFC3339)
	s.st.Devices[deviceID] = d
	if err := s.saveLocked(); err != nil {
		return VerifyUnauthorized, false, err
	}
	return VerifyActive, true, nil
}

func (s *RelayStore) SetPush(deviceID, token, platform string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	d, ok := s.st.Devices[deviceID]
	if !ok {
		return errors.New("device not found")
	}
	// A new/changed token invalidates any gateway credential bound to the old
	// token: clear the key_id and registration state so the device re-registers.
	if d.PushToken != token {
		d.PushKeyID = ""
		d.PushRegState = ""
	}
	d.PushToken = token
	d.PushPlatform = platform
	s.st.Devices[deviceID] = d
	return s.saveLocked()
}

// SetPushKeyID records the gateway credential id returned by a successful
// registration complete() and marks the device's push registration active.
func (s *RelayStore) SetPushKeyID(deviceID, keyID string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	d, ok := s.st.Devices[deviceID]
	if !ok {
		return errors.New("device not found")
	}
	d.PushKeyID = keyID
	d.PushRegState = "active"
	s.st.Devices[deviceID] = d
	return s.saveLocked()
}

// SetDevicePushPreview records this device's own push-preview preference (FB23):
// clear=true wants the full message text in its push body, clear=false wants the
// generic "New Message". Persisting a concrete value (not nil) marks the
// preference explicit, so the wake path honors it over the box env default. A
// missing device is not an error (nothing to record).
func (s *RelayStore) SetDevicePushPreview(deviceID string, clear bool) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	d, ok := s.st.Devices[deviceID]
	if !ok {
		return nil
	}
	d.PushPreviewClear = &clear
	s.st.Devices[deviceID] = d
	return s.saveLocked()
}

// SetDeviceJobCompletionPush records this device's FB44 successful-job push
// preference. Persisting a concrete bool distinguishes an explicit choice from
// nil, whose additive default is enabled. A missing device is not an error.
func (s *RelayStore) SetDeviceJobCompletionPush(deviceID string, enabled bool) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	d, ok := s.st.Devices[deviceID]
	if !ok {
		return nil
	}
	d.JobCompletionPush = &enabled
	s.st.Devices[deviceID] = d
	return s.saveLocked()
}

// SetLiveActivity registers or replaces one job's ActivityKit token for an
// active device. Adding a 33rd distinct job evicts the oldest registration.
func (s *RelayStore) SetLiveActivity(deviceID, jobID, token string) error {
	return s.setLiveActivityAt(deviceID, jobID, token, time.Now().UTC())
}

func (s *RelayStore) setLiveActivityAt(deviceID, jobID, token string, now time.Time) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	d, ok := s.st.Devices[deviceID]
	if !ok || d.State != DeviceActive {
		return errors.New("active device not found")
	}
	r, ok := s.st.Rooms[d.Room]
	if !ok || r.State == RoomDead {
		return errors.New("active device room not found")
	}
	if d.LiveActivities == nil {
		d.LiveActivities = make(map[string]LiveActivityRegistration)
	}
	if _, exists := d.LiveActivities[jobID]; !exists && len(d.LiveActivities) >= maxLiveActivitiesPerDevice {
		oldestID := ""
		var oldestTime time.Time
		for id, reg := range d.LiveActivities {
			registeredAt, err := time.Parse(liveActivityRegisteredAtTime, reg.RegisteredAt)
			if err != nil {
				registeredAt = time.Time{}
			}
			if oldestID == "" || registeredAt.Before(oldestTime) || (registeredAt.Equal(oldestTime) && id < oldestID) {
				oldestID = id
				oldestTime = registeredAt
			}
		}
		delete(d.LiveActivities, oldestID)
	}
	d.LiveActivities[jobID] = LiveActivityRegistration{
		Token:        token,
		RegisteredAt: now.Format(liveActivityRegisteredAtTime),
	}
	s.st.Devices[deviceID] = d
	return s.saveLocked()
}

// RemoveLiveActivity idempotently unregisters one job from one device.
func (s *RelayStore) RemoveLiveActivity(deviceID, jobID string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	d, ok := s.st.Devices[deviceID]
	if !ok || d.LiveActivities == nil {
		return nil
	}
	if _, exists := d.LiveActivities[jobID]; !exists {
		return nil
	}
	delete(d.LiveActivities, jobID)
	if len(d.LiveActivities) == 0 {
		d.LiveActivities = nil
	}
	s.st.Devices[deviceID] = d
	return s.saveLocked()
}

// ActiveLiveActivityTargets snapshots every active, live-room device currently
// registered for jobID. The deterministic order is useful to lifecycle callers
// that synchronously enqueue an immutable request for each target.
func (s *RelayStore) ActiveLiveActivityTargets(jobID string) []LiveActivityTarget {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	var out []LiveActivityTarget
	for _, d := range s.st.Devices {
		if d.State != DeviceActive {
			continue
		}
		r, roomOK := s.st.Rooms[d.Room]
		if !roomOK || r.State == RoomDead {
			continue
		}
		if reg, ok := d.LiveActivities[jobID]; ok {
			out = append(out, LiveActivityTarget{DeviceID: d.ID, JobID: jobID, Token: reg.Token})
		}
	}
	sort.Slice(out, func(i, j int) bool { return out[i].DeviceID < out[j].DeviceID })
	return out
}

// TakeLiveActivityTargets atomically snapshots and removes every registration
// for jobID. Terminal lifecycle code uses the returned tokens for one final end
// event after the durable clear has succeeded.
func (s *RelayStore) TakeLiveActivityTargets(jobID string) ([]LiveActivityTarget, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	var out []LiveActivityTarget
	for id, d := range s.st.Devices {
		reg, ok := d.LiveActivities[jobID]
		if !ok {
			continue
		}
		out = append(out, LiveActivityTarget{DeviceID: d.ID, JobID: jobID, Token: reg.Token})
		delete(d.LiveActivities, jobID)
		if len(d.LiveActivities) == 0 {
			d.LiveActivities = nil
		}
		s.st.Devices[id] = d
	}
	if len(out) == 0 {
		return nil, nil
	}
	sort.Slice(out, func(i, j int) bool { return out[i].DeviceID < out[j].DeviceID })
	if err := s.saveLocked(); err != nil {
		return nil, err
	}
	return out, nil
}

// DropLiveActivityIfToken conditionally removes a registration only when it
// still carries the token APNs rejected. A replacement registered while the old
// request was in flight is therefore preserved.
func (s *RelayStore) DropLiveActivityIfToken(deviceID, jobID, token string) (bool, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	d, ok := s.st.Devices[deviceID]
	if !ok {
		return false, nil
	}
	reg, ok := d.LiveActivities[jobID]
	if !ok || reg.Token != token {
		return false, nil
	}
	delete(d.LiveActivities, jobID)
	if len(d.LiveActivities) == 0 {
		d.LiveActivities = nil
	}
	s.st.Devices[deviceID] = d
	if err := s.saveLocked(); err != nil {
		return false, err
	}
	return true, nil
}

// DropPushToken permanently removes a device's push token and gateway
// credential after a terminal APNs rejection (410 / token_invalid /
// drop_token). A missing device is not an error (nothing to drop).
func (s *RelayStore) DropPushToken(deviceID string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	d, ok := s.st.Devices[deviceID]
	if !ok {
		return nil
	}
	d.PushToken = ""
	d.PushKeyID = ""
	d.PushRegState = "dropped"
	s.st.Devices[deviceID] = d
	return s.saveLocked()
}

func (s *RelayStore) Device(deviceID string) (DeviceRecord, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	d, ok := s.st.Devices[deviceID]
	return d, ok
}

// ActivePushTarget returns the device's push token together with its OWN bound
// room (SPEC §5/MD3), read atomically under the store lock, and only when the
// device is currently active and its room is live (open|bound, not dead).
// Resolving the room via the device's binding — instead of the global
// current_room — is what makes push per-device across N concurrently served
// rooms. Reading the token and the room in a single locked snapshot preserves
// the atomic-snapshot property: no concurrent mint/revoke can pair a token with
// a stale room label.
func (s *RelayStore) ActivePushTarget(deviceID string) (token, keyID, room string, ok bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	d, exists := s.st.Devices[deviceID]
	if !exists || d.State != DeviceActive {
		return "", "", "", false
	}
	r, roomOK := s.st.Rooms[d.Room]
	if !roomOK || r.State == RoomDead {
		return "", "", "", false
	}
	return d.PushToken, d.PushKeyID, d.Room, true
}

func (s *RelayStore) Revoke(id string) (DeviceRecord, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	var matches []string
	for key := range s.st.Devices {
		if key == id || strings.HasPrefix(key, id) {
			matches = append(matches, key)
		}
	}
	if len(matches) != 1 {
		if len(matches) == 0 {
			return DeviceRecord{}, fmt.Errorf("device %q not found", id)
		}
		return DeviceRecord{}, fmt.Errorf("device prefix %q is ambiguous", id)
	}
	d := s.st.Devices[matches[0]]
	d.State = DeviceBanned
	d.LiveActivities = nil
	s.st.Devices[matches[0]] = d
	// Free the slot: the device's bound room becomes a dead tombstone — never
	// dialed, never registered, never counted against the cap (SPEC §2.3). The
	// caller uses the returned d.Room for the best-effort core DELETE (fixing the
	// latent CurrentRoom bug).
	if r, ok := s.st.Rooms[d.Room]; ok && r.State != RoomDead {
		r.State = RoomDead
		s.st.Rooms[d.Room] = r
	}
	if err := s.saveLocked(); err != nil {
		return DeviceRecord{}, err
	}
	return d, nil
}

// RevokeResolution classifies a `relay revoke <arg>` target so the caller can
// route to the right kill path. Kind is "device" or "room"; ID is the full
// matched id.
type RevokeResolution struct {
	Kind string
	ID   string
}

// ResolveRevoke classifies a revoke argument against BOTH the device roster and
// the open-room set, accepting a unique prefix like the CLI's other id args
// (FB27). Devices and non-dead rooms share one prefix namespace: exactly one
// match total resolves; zero or more than one is an error (including a
// cross-kind prefix collision). A room that a live device rides is refused with
// guidance to revoke the device instead — a bound room must never be nuked by
// room-id without its device.
func (s *RelayStore) ResolveRevoke(arg string) (RevokeResolution, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()

	var devMatches, roomMatches []string
	for id := range s.st.Devices {
		if id == arg || strings.HasPrefix(id, arg) {
			devMatches = append(devMatches, id)
		}
	}
	for id, r := range s.st.Rooms {
		if r.State == RoomDead {
			continue
		}
		if id == arg || strings.HasPrefix(id, arg) {
			roomMatches = append(roomMatches, id)
		}
	}

	switch total := len(devMatches) + len(roomMatches); {
	case total == 0:
		return RevokeResolution{}, fmt.Errorf("no device or open room matching %q", arg)
	case total > 1:
		return RevokeResolution{}, fmt.Errorf("%q is ambiguous: matches %d device(s) and %d room(s); use a longer prefix or the full id", arg, len(devMatches), len(roomMatches))
	case len(devMatches) == 1:
		return RevokeResolution{Kind: "device", ID: devMatches[0]}, nil
	}

	roomID := roomMatches[0]
	for _, d := range s.st.Devices {
		if d.Room == roomID && d.State == DeviceActive {
			return RevokeResolution{}, fmt.Errorf("room %s is bound to device %s; revoke the device instead: hotline relay revoke %s", shortRoomID(roomID), d.ID, d.ID)
		}
	}
	return RevokeResolution{Kind: "room", ID: roomID}, nil
}

// RevokeRoom kills an OPEN (unredeemed) room by its full id (FB27): the record is
// deleted locally, freeing the slot it squatted. It refuses a room a live device
// rides — that path must go through device-id Revoke — so a bound room is never
// nuked out from under its device. The relay expires the room server-side, so no
// remote unregister call is needed (hotline-core exposes no room-delete control
// action).
func (s *RelayStore) RevokeRoom(id string) (RoomRecord, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reloadLocked()
	r, ok := s.st.Rooms[id]
	if !ok || r.State == RoomDead {
		return RoomRecord{}, fmt.Errorf("open room %q not found", id)
	}
	for _, d := range s.st.Devices {
		if d.Room == id && d.State == DeviceActive {
			return RoomRecord{}, fmt.Errorf("room %s is bound to device %s; revoke the device instead: hotline relay revoke %s", shortRoomID(id), d.ID, d.ID)
		}
	}
	delete(s.st.Rooms, id)
	if s.st.CurrentRoom == id {
		s.st.CurrentRoom = ""
	}
	if err := s.saveLocked(); err != nil {
		return RoomRecord{}, err
	}
	return r, nil
}

// shortRoomID trims a room id to its display prefix (matches relay status).
func shortRoomID(id string) string {
	if len(id) > 8 {
		return id[:8]
	}
	return id
}

func (s *RelayStore) reloadLocked() {
	data, err := os.ReadFile(s.path)
	if err != nil {
		return
	}
	var fresh relayState
	if json.Unmarshal(data, &fresh) != nil {
		return
	}
	if fresh.Rooms == nil {
		fresh.Rooms = map[string]RoomRecord{}
	}
	if fresh.Devices == nil {
		fresh.Devices = map[string]DeviceRecord{}
	}
	s.st = fresh
}

func (s *RelayStore) saveLocked() error {
	if err := os.MkdirAll(filepath.Dir(s.path), 0o700); err != nil {
		return err
	}
	data, err := json.MarshalIndent(s.st, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')
	tmp, err := os.CreateTemp(filepath.Dir(s.path), ".relay-state-*")
	if err != nil {
		return err
	}
	name := tmp.Name()
	defer os.Remove(name)
	if err := tmp.Chmod(0o600); err != nil {
		tmp.Close()
		return err
	}
	if _, err := tmp.Write(data); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Sync(); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	if err := os.Rename(name, s.path); err != nil {
		return err
	}
	return os.Chmod(s.path, 0o600)
}
