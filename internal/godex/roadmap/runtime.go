package roadmap

import (
	"context"
	"fmt"
	"path/filepath"
	"strings"

	"github.com/google/uuid"
	"github.com/pandelisz/gode/internal/godex/eventbus"
)

type Runtime struct {
	Workspace string
	DataDir   string
	Bus       *eventbus.Bus
}

func NewRuntime(workspace, dataDir string, bus *eventbus.Bus) *Runtime {
	return &Runtime{Workspace: workspace, DataDir: dataDir, Bus: bus}
}

func (r *Runtime) ListDocuments(ctx context.Context) ([]DocumentSummary, error) {
	return ListDocuments(r.Workspace, false)
}

func (r *Runtime) Open(ctx context.Context, path string) (*Document, error) {
	resolved, err := r.resolvePath(path)
	if err != nil {
		return nil, err
	}
	doc, err := ParseFile(resolved)
	if err != nil {
		return nil, err
	}
	state, err := LoadState(r.DataDir)
	if err != nil {
		return nil, err
	}
	state.FocusedDocument = resolved
	if err := SaveState(r.DataDir, state); err != nil {
		return nil, err
	}
	r.publish(ctx, eventbus.KindRoadmapOpened, map[string]any{"path": resolved, "title": doc.Title})
	return &doc, nil
}

func (r *Runtime) FocusTask(ctx context.Context, path string, taskID string) error {
	resolved, err := r.resolvePath(path)
	if err != nil {
		return err
	}
	if _, err := r.taskByID(resolved, taskID); err != nil {
		return err
	}
	state, err := LoadState(r.DataDir)
	if err != nil {
		return err
	}
	state.FocusedDocument = resolved
	state.FocusedTaskID = taskID
	if err := SaveState(r.DataDir, state); err != nil {
		return err
	}
	r.publish(ctx, eventbus.KindRoadmapTaskFocused, map[string]any{"path": resolved, "task_id": taskID})
	return nil
}

func (r *Runtime) SetTask(ctx context.Context, path string, taskID string, checked bool, evidence string) error {
	resolved, err := r.resolvePath(path)
	if err != nil {
		return err
	}
	if err := SetTaskChecked(resolved, taskID, checked, evidence); err != nil {
		return err
	}
	r.publish(ctx, eventbus.KindRoadmapTaskChanged, map[string]any{"path": resolved, "task_id": taskID, "checked": checked, "evidence": evidence})
	r.publish(ctx, eventbus.KindRoadmapUpdated, map[string]any{"path": resolved})
	return nil
}

func (r *Runtime) Validate(ctx context.Context, path string) (ValidationResult, error) {
	resolved, err := r.resolvePath(path)
	if err != nil {
		return ValidationResult{}, err
	}
	doc, err := ParseFile(resolved)
	if err != nil {
		return ValidationResult{}, err
	}
	result := ValidationResult{Path: resolved, Diagnostics: Validate(doc)}
	r.publish(ctx, eventbus.KindRoadmapValidated, map[string]any{"path": resolved, "diagnostic_count": len(result.Diagnostics)})
	return result, nil
}

func (r *Runtime) ContextPrompt(ctx context.Context, path string) (string, error) {
	resolved, err := r.resolvePath(path)
	if err != nil {
		return "", err
	}
	doc, err := ParseFile(resolved)
	if err != nil {
		return "", err
	}
	state, err := LoadState(r.DataDir)
	if err != nil {
		return "", err
	}
	var focused *Task
	if state.FocusedDocument == resolved && state.FocusedTaskID != "" {
		for i := range doc.Tasks {
			if doc.Tasks[i].ID == state.FocusedTaskID {
				focused = &doc.Tasks[i]
				break
			}
		}
	}
	skillBody, err := LoadPlanningSkillBody(r.Workspace)
	if err != nil {
		return "", err
	}
	validation := ValidationResult{Path: resolved, Diagnostics: Validate(doc)}
	return RoadmapContextPrompt(doc, focused, validation, skillBody), nil
}

func (r *Runtime) ListThreads(ctx context.Context, path string) ([]ThreadAttachment, error) {
	resolved, err := r.resolvePath(path)
	if err != nil {
		return nil, err
	}
	state, err := LoadState(r.DataDir)
	if err != nil {
		return nil, err
	}
	var out []ThreadAttachment
	for _, attachment := range state.AttachedThreads {
		if attachment.Path == resolved {
			out = append(out, attachment)
		}
	}
	return out, nil
}

func (r *Runtime) SpawnThread(ctx context.Context, path string, taskID string) (ThreadAttachment, error) {
	return r.attachThread(ctx, path, taskID, uuid.NewString(), eventbus.KindRoadmapThreadSpawned)
}

func (r *Runtime) AttachThread(ctx context.Context, path string, taskID string, threadID string) (ThreadAttachment, error) {
	return r.attachThread(ctx, path, taskID, threadID, eventbus.KindRoadmapThreadAttached)
}

func (r *Runtime) attachThread(ctx context.Context, path string, taskID string, threadID string, kind eventbus.Kind) (ThreadAttachment, error) {
	if threadID == "" {
		return ThreadAttachment{}, fmt.Errorf("thread id is required")
	}
	resolved, err := r.resolvePath(path)
	if err != nil {
		return ThreadAttachment{}, err
	}
	if taskID != "" {
		if _, err := r.taskByID(resolved, taskID); err != nil {
			return ThreadAttachment{}, err
		}
	}
	state, err := LoadState(r.DataDir)
	if err != nil {
		return ThreadAttachment{}, err
	}
	attachment := ThreadAttachment{Path: resolved, TaskID: taskID, ThreadID: threadID}
	state.AttachedThreads = append(state.AttachedThreads, attachment)
	if err := SaveState(r.DataDir, state); err != nil {
		return ThreadAttachment{}, err
	}
	r.publish(ctx, kind, map[string]any{"path": resolved, "task_id": taskID, "thread_id": threadID})
	return attachment, nil
}

func (r *Runtime) ModeChanged(ctx context.Context, active bool) {
	r.publish(ctx, eventbus.KindRoadmapModeChanged, map[string]any{"active": active})
}

func (r *Runtime) taskByID(path string, taskID string) (Task, error) {
	doc, err := ParseFile(path)
	if err != nil {
		return Task{}, err
	}
	for _, task := range doc.Tasks {
		if task.ID == taskID {
			return task, nil
		}
	}
	return Task{}, fmt.Errorf("task %s not found", taskID)
}

func (r *Runtime) resolvePath(path string) (string, error) {
	if r == nil {
		return "", fmt.Errorf("roadmap runtime is nil")
	}
	if path == "" {
		return "", fmt.Errorf("roadmap path is required")
	}
	if !filepath.IsAbs(path) {
		path = filepath.Join(r.Workspace, path)
	}
	clean, err := filepath.Abs(filepath.Clean(path))
	if err != nil {
		return "", err
	}
	roadmapDir, err := filepath.Abs(filepath.Join(r.Workspace, "roadmap"))
	if err != nil {
		return "", err
	}
	rel, err := filepath.Rel(roadmapDir, clean)
	if err != nil || rel == "." || rel == ".." || rel == "" || strings.HasPrefix(rel, ".."+string(filepath.Separator)) {
		return "", fmt.Errorf("roadmap path must be under %s", roadmapDir)
	}
	if filepath.Ext(clean) != ".md" {
		return "", fmt.Errorf("roadmap path must be a markdown file")
	}
	return clean, nil
}

func (r *Runtime) publish(ctx context.Context, kind eventbus.Kind, payload map[string]any) {
	if r == nil || r.Bus == nil {
		return
	}
	r.Bus.Publish(ctx, eventbus.NewEvent(kind, eventbus.SourceAgent, payload))
}
