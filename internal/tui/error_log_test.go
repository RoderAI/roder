package tui

import (
	"errors"
	"strings"
	"testing"
)

func TestTimelineErrorIsSummarizedButErrorLogKeepsFullDetail(t *testing.T) {
	full := strings.Join([]string{
		"OpenAI stream request failed",
		"request: POST https://chatgpt.com/backend-api/codex/responses",
		"status: 400 Bad Request",
		"x-request-id: req_123",
		"error_type: invalid_request_error",
		"error_message: bad request",
		"response_body:",
		`{"detail":"unsupported model for codex subscription"}`,
		"model: gpt-5.5",
	}, "\n")

	updated, _ := New(nil).Update(runDoneMsg{Err: errors.New(full)})
	got := updated.(Model)

	if len(got.messages) != 1 {
		t.Fatalf("messages length = %d, want 1", len(got.messages))
	}
	timeline := got.messages[0].Body
	if !strings.Contains(timeline, "400 Bad Request") || !strings.Contains(timeline, "ctrl+l for details") {
		t.Fatalf("timeline error should be a useful summary, got %q", timeline)
	}
	if strings.Contains(timeline, "response_body") || strings.Contains(timeline, "unsupported model for codex subscription") {
		t.Fatalf("timeline error should not include full diagnostics, got %q", timeline)
	}

	if len(got.errorLog) != 1 {
		t.Fatalf("error log length = %d, want 1", len(got.errorLog))
	}
	if got.errorLog[0].Message != full {
		t.Fatalf("error log should keep full detail:\n%s", got.errorLog[0].Message)
	}
}

func TestRunDoneDedupeUsesFullErrorLogMessage(t *testing.T) {
	full := "OpenAI stream request failed\nstatus: 400 Bad Request\nresponse_body:\n{}"
	model := New(nil)
	updated, _ := model.Update(runDoneMsg{Err: errors.New(full)})
	updated, _ = updated.Update(runDoneMsg{Err: errors.New(full)})
	got := updated.(Model)

	if len(got.messages) != 1 {
		t.Fatalf("messages length = %d, want 1", len(got.messages))
	}
	if len(got.errorLog) != 1 {
		t.Fatalf("error log length = %d, want 1", len(got.errorLog))
	}
}
