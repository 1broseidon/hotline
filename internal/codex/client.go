// Package codex adapts `codex app-server` to hotline's harness.Link seam. The
// app-server transport is newline-delimited JSON-RPC over stdio: hotline owns
// the subprocess, sends thread/turn requests, receives notifications and
// server-initiated approval requests, and answers those requests on the same
// stream.
package codex

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"sync"
	"time"
)

const defaultBinary = "codex"

// Client is a minimal JSONL JSON-RPC client for codex app-server.
type Client struct {
	rwc io.ReadWriteCloser

	writeMu sync.Mutex

	mu      sync.Mutex
	nextID  int64
	pending map[int64]chan rpcResponse
	closed  bool

	requests      chan ServerRequest
	notifications chan Notification
	done          chan error
}

type rpcMessage struct {
	ID     *int64          `json:"id,omitempty"`
	Method string          `json:"method,omitempty"`
	Params json.RawMessage `json:"params,omitempty"`
	Result json.RawMessage `json:"result,omitempty"`
	Error  *rpcError       `json:"error,omitempty"`
}

type rpcError struct {
	Code    int             `json:"code,omitempty"`
	Message string          `json:"message"`
	Data    json.RawMessage `json:"data,omitempty"`
}

type rpcResponse struct {
	result json.RawMessage
	err    error
}

// ServerRequest is a server-initiated JSON-RPC request, such as a command
// approval prompt. The caller must answer it with Respond or RespondError.
type ServerRequest struct {
	ID     int64
	Method string
	Params json.RawMessage
}

// Notification is a server notification.
type Notification struct {
	Method string
	Params json.RawMessage
}

// NewClient wraps an already-open JSONL read/write stream. Tests use this seam
// to exercise Link without a real codex binary.
func NewClient(rwc io.ReadWriteCloser) *Client {
	c := &Client{
		rwc:           rwc,
		nextID:        1,
		pending:       make(map[int64]chan rpcResponse),
		requests:      make(chan ServerRequest, 16),
		notifications: make(chan Notification, 128),
		done:          make(chan error, 1),
	}
	go c.readLoop()
	return c
}

// StartAppServer spawns `codex app-server` in cwd and returns a connected
// client. The child is Pdeathsig-guarded on Linux so it dies with hotline if the
// parent process is severed.
func StartAppServer(cwd string) (*Client, error) {
	bin, err := exec.LookPath(defaultBinary)
	if err != nil {
		return nil, fmt.Errorf("codex not found on PATH")
	}
	cmd := exec.Command(bin, "app-server")
	cmd.Dir = cwd
	setPdeathsig(cmd)

	stdin, err := cmd.StdinPipe()
	if err != nil {
		return nil, err
	}
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return nil, err
	}
	stderr, err := cmd.StderrPipe()
	if err != nil {
		return nil, err
	}
	if err := cmd.Start(); err != nil {
		return nil, err
	}
	go drainStderr(stderr)

	conn := &processConn{cmd: cmd, stdin: stdin, stdout: stdout}
	return NewClient(conn), nil
}

func drainStderr(r io.Reader) {
	sc := bufio.NewScanner(r)
	sc.Buffer(make([]byte, 0, 64*1024), 1024*1024)
	for sc.Scan() {
		fmt.Fprintf(os.Stderr, "hotline: codex: %s\n", sc.Text())
	}
}

// Requests returns the stream of server-initiated requests.
func (c *Client) Requests() <-chan ServerRequest { return c.requests }

// Notifications returns the stream of server notifications.
func (c *Client) Notifications() <-chan Notification { return c.notifications }

// Done is closed when the JSONL reader stops. The received error is nil for a
// local Close or EOF, non-nil for malformed JSON or a transport read error.
func (c *Client) Done() <-chan error { return c.done }

// Initialize performs the app-server initialize/initialized handshake.
func (c *Client) Initialize(ctx context.Context) error {
	params := map[string]any{
		"clientInfo": map[string]any{
			"title":   "Hotline",
			"name":    "hotline",
			"version": "0.1.0",
		},
		"capabilities": map[string]any{
			"experimentalApi":                true,
			"requestAttestation":             false,
			"mcpServerOpenaiFormElicitation": false,
			"optOutNotificationMethods":      []string{},
		},
	}
	if err := c.Request(ctx, "initialize", params, nil); err != nil {
		return err
	}
	return c.Notify(ctx, "initialized", map[string]any{})
}

// Request sends a client-initiated request and decodes the result into out when
// non-nil.
func (c *Client) Request(ctx context.Context, method string, params any, out any) error {
	raw, err := json.Marshal(params)
	if err != nil {
		return err
	}

	id, ch, err := c.register()
	if err != nil {
		return err
	}
	msg := rpcMessage{ID: &id, Method: method, Params: raw}
	if err := c.send(msg); err != nil {
		c.unregister(id)
		return err
	}

	select {
	case res := <-ch:
		if res.err != nil {
			return res.err
		}
		if out == nil || len(res.result) == 0 {
			return nil
		}
		return json.Unmarshal(res.result, out)
	case <-ctx.Done():
		c.unregister(id)
		return ctx.Err()
	}
}

// Notify sends a client notification.
func (c *Client) Notify(ctx context.Context, method string, params any) error {
	raw, err := json.Marshal(params)
	if err != nil {
		return err
	}
	done := make(chan error, 1)
	go func() {
		done <- c.send(rpcMessage{Method: method, Params: raw})
	}()
	select {
	case err := <-done:
		return err
	case <-ctx.Done():
		return ctx.Err()
	}
}

// Respond answers a server-initiated request.
func (c *Client) Respond(ctx context.Context, id int64, result any) error {
	raw, err := json.Marshal(result)
	if err != nil {
		return err
	}
	done := make(chan error, 1)
	go func() { done <- c.send(rpcMessage{ID: &id, Result: raw}) }()
	select {
	case err := <-done:
		return err
	case <-ctx.Done():
		return ctx.Err()
	}
}

// RespondError rejects a server-initiated request.
func (c *Client) RespondError(ctx context.Context, id int64, code int, message string) error {
	errMsg := &rpcError{Code: code, Message: message}
	done := make(chan error, 1)
	go func() { done <- c.send(rpcMessage{ID: &id, Error: errMsg}) }()
	select {
	case err := <-done:
		return err
	case <-ctx.Done():
		return ctx.Err()
	}
}

// Close closes the underlying stream and wakes all pending requests.
func (c *Client) Close() error {
	c.mu.Lock()
	if c.closed {
		c.mu.Unlock()
		return nil
	}
	c.closed = true
	c.mu.Unlock()
	return c.rwc.Close()
}

func (c *Client) register() (int64, chan rpcResponse, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.closed {
		return 0, nil, fmt.Errorf("codex app-server client is closed")
	}
	id := c.nextID
	c.nextID++
	ch := make(chan rpcResponse, 1)
	c.pending[id] = ch
	return id, ch, nil
}

func (c *Client) unregister(id int64) {
	c.mu.Lock()
	delete(c.pending, id)
	c.mu.Unlock()
}

func (c *Client) send(msg rpcMessage) error {
	raw, err := json.Marshal(msg)
	if err != nil {
		return err
	}
	c.writeMu.Lock()
	defer c.writeMu.Unlock()
	if _, err := c.rwc.Write(append(raw, '\n')); err != nil {
		return err
	}
	return nil
}

func (c *Client) readLoop() {
	err := c.scan()
	c.mu.Lock()
	c.closed = true
	for _, ch := range c.pending {
		ch <- rpcResponse{err: fmt.Errorf("codex app-server connection closed")}
	}
	c.pending = map[int64]chan rpcResponse{}
	c.mu.Unlock()
	close(c.requests)
	close(c.notifications)
	c.done <- err
	close(c.done)
}

func (c *Client) scan() error {
	sc := bufio.NewScanner(c.rwc)
	sc.Buffer(make([]byte, 0, 64*1024), 64*1024*1024)
	for sc.Scan() {
		line := sc.Bytes()
		if len(line) == 0 {
			continue
		}
		var msg rpcMessage
		if err := json.Unmarshal(line, &msg); err != nil {
			return fmt.Errorf("decoding codex app-server JSONL: %w", err)
		}
		c.dispatch(msg)
	}
	if err := sc.Err(); err != nil {
		return err
	}
	return nil
}

func (c *Client) dispatch(msg rpcMessage) {
	if msg.ID != nil && msg.Method != "" {
		c.requests <- ServerRequest{ID: *msg.ID, Method: msg.Method, Params: msg.Params}
		return
	}
	if msg.ID != nil {
		c.mu.Lock()
		ch := c.pending[*msg.ID]
		delete(c.pending, *msg.ID)
		c.mu.Unlock()
		if ch == nil {
			return
		}
		if msg.Error != nil {
			ch <- rpcResponse{err: fmt.Errorf("codex app-server: %s", msg.Error.Message)}
		} else {
			ch <- rpcResponse{result: msg.Result}
		}
		return
	}
	if msg.Method != "" {
		c.notifications <- Notification{Method: msg.Method, Params: msg.Params}
	}
}

type processConn struct {
	cmd    *exec.Cmd
	stdin  io.WriteCloser
	stdout io.ReadCloser
	once   sync.Once
}

func (p *processConn) Read(b []byte) (int, error)  { return p.stdout.Read(b) }
func (p *processConn) Write(b []byte) (int, error) { return p.stdin.Write(b) }

func (p *processConn) Close() error {
	var err error
	p.once.Do(func() {
		err = p.stdin.Close()
		time.AfterFunc(50*time.Millisecond, func() {
			if p.cmd.Process != nil {
				_ = p.cmd.Process.Signal(os.Interrupt)
				time.AfterFunc(200*time.Millisecond, func() {
					if p.cmd.Process != nil {
						_ = p.cmd.Process.Kill()
					}
				})
			}
		})
		go func() {
			_ = p.cmd.Wait()
			_ = p.stdout.Close()
		}()
	})
	return err
}
