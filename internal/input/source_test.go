package input

import (
	"os"
	"path/filepath"
	"testing"
)

func TestResolveSource_File(t *testing.T) {
	dir := t.TempDir()
	p := filepath.Join(dir, "a.md")
	if err := os.WriteFile(p, []byte("# hello\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	src, err := ResolveSource([]string{p}, os.Stdin)
	if err != nil {
		t.Fatal(err)
	}
	if src == nil {
		t.Fatalf("expected source")
	}
	b, err := src.ReadAll()
	if err != nil {
		t.Fatal(err)
	}
	if string(b) != "# hello\n" {
		t.Fatalf("unexpected content: %q", string(b))
	}
}
