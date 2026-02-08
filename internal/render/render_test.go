package render

import "testing"

func TestRenderMarkdown_NonEmpty(t *testing.T) {
	out, err := RenderMarkdown("# Title\n\nHello\n", Options{Style: "auto", Width: 60})
	if err != nil {
		t.Fatal(err)
	}
	if out == "" {
		t.Fatalf("expected non-empty output")
	}
}
