package roadmap

type Document struct {
	Path               string
	Title              string
	Goal               string
	Architecture       string
	TechStack          string
	Tasks              []Task
	RunBlocks          []RunBlock
	AcceptanceSections []Section
	Lines              []string
	Raw                string
}

type DocumentSummary struct {
	Path      string `json:"path"`
	Title     string `json:"title"`
	Checked   int    `json:"checked"`
	Unchecked int    `json:"unchecked"`
}

type Task struct {
	ID      string
	Text    string
	Checked bool
	Line    int
}

type RunBlock struct {
	Line int
	Text string
}

type Section struct {
	Line  int
	Title string
}

type Diagnostic struct {
	Path     string
	Line     int
	Severity string
	Message  string
}

type ValidationResult struct {
	Path        string       `json:"path"`
	Diagnostics []Diagnostic `json:"diagnostics"`
}

type ThreadAttachment struct {
	Path     string `json:"path"`
	TaskID   string `json:"taskId,omitempty"`
	ThreadID string `json:"threadId"`
}
