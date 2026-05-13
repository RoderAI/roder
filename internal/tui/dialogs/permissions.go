package dialogs

type PermissionChoice string

const (
	PermissionAllow           PermissionChoice = "allow"
	PermissionDeny            PermissionChoice = "deny"
	PermissionAllowForSession PermissionChoice = "allow_for_session"
)

type PermissionRequest struct {
	ID            string
	CorrelationID string
	SessionID     string
	RunID         string
	Tool          string
	Action        string
	Input         string
	Selected      bool
}

type Permissions struct {
	Open     bool
	Requests []PermissionRequest
	Selected int
	Err      string
}

func NewPermissions(requests []PermissionRequest) Permissions {
	out := Permissions{Open: true, Requests: append([]PermissionRequest(nil), requests...)}
	out.markSelected()
	return out
}

func (p *Permissions) Move(delta int) {
	if len(p.Requests) == 0 {
		return
	}
	p.Selected = wrapIndex(p.Selected+delta, len(p.Requests))
	p.markSelected()
}

func (p Permissions) SelectedRequest() PermissionRequest {
	if len(p.Requests) == 0 {
		return PermissionRequest{}
	}
	return p.Requests[clamp(p.Selected, 0, len(p.Requests)-1)]
}

func (p *Permissions) RemoveSelected() PermissionRequest {
	selected := p.SelectedRequest()
	if len(p.Requests) == 0 {
		return selected
	}
	index := clamp(p.Selected, 0, len(p.Requests)-1)
	p.Requests = append(p.Requests[:index], p.Requests[index+1:]...)
	if p.Selected >= len(p.Requests) {
		p.Selected = max(0, len(p.Requests)-1)
	}
	p.markSelected()
	if len(p.Requests) == 0 {
		p.Open = false
	}
	return selected
}

func (p *Permissions) markSelected() {
	for i := range p.Requests {
		p.Requests[i].Selected = i == p.Selected
	}
}
