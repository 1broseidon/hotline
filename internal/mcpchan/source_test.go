package mcpchan

import (
	"encoding/json"
	"strings"
	"testing"
)

// TestWithSourcePropertySingleProviderUnchanged pins backward compatibility:
// with zero or one provider the schemas are byte-identical to the originals —
// no source property leaks into the single-provider tool surface.
func TestWithSourcePropertySingleProviderUnchanged(t *testing.T) {
	for _, schema := range []string{replySchema, reactSchema, editSchema, downloadSchema} {
		if got := withSourceProperty(schema, nil); got != schema {
			t.Errorf("nil sources must not change the schema")
		}
		if got := withSourceProperty(schema, []string{"telegram"}); got != schema {
			t.Errorf("a single source must not change the schema")
		}
	}
}

// TestWithSourcePropertyMultiProvider proves multi-provider schemas grow a
// required source property enumerating the configured providers.
func TestWithSourcePropertyMultiProvider(t *testing.T) {
	sources := []string{"telegram", "discord"}
	for _, schema := range []string{replySchema, reactSchema, editSchema, downloadSchema} {
		out := withSourceProperty(schema, sources)
		var m struct {
			Properties map[string]struct {
				Type string   `json:"type"`
				Enum []string `json:"enum"`
			} `json:"properties"`
			Required []string `json:"required"`
		}
		if err := json.Unmarshal([]byte(out), &m); err != nil {
			t.Fatalf("injected schema is not valid JSON: %v\n%s", err, out)
		}
		src, ok := m.Properties["source"]
		if !ok {
			t.Fatalf("source property missing: %s", out)
		}
		if src.Type != "string" || strings.Join(src.Enum, ",") != "telegram,discord" {
			t.Errorf("source property = %+v", src)
		}
		foundReq := false
		for _, r := range m.Required {
			if r == "source" {
				foundReq = true
			}
		}
		if !foundReq {
			t.Errorf("source should be required with multiple providers: %v", m.Required)
		}
		// The original required fields must survive.
		if len(m.Required) < 2 && schema != replySchema {
			t.Errorf("original required fields lost: %v", m.Required)
		}
	}
}
