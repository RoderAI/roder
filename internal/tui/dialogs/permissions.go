package dialogs

type PermissionChoice string

const (
	PermissionAllow           PermissionChoice = "allow"
	PermissionDeny            PermissionChoice = "deny"
	PermissionAllowForSession PermissionChoice = "allow_for_session"
)

type PermissionRequest struct {
	ID       string
	Tool     string
	Action   string
	Input    string
	Selected bool
}

type Permissions struct {
	Open     bool
	Requests []PermissionRequest
	Selected int
	Err      string
}

func NewPermissions(requests []PermissionRequest) Permissions {
	return Permissions{Open: true, Requests: append([]PermissionRequest(nil), requests...)}
}

func (p *Permissions) Move(delta int) {
	if len(p.Requests) == 0 {
		return
	}
	p.Selected = wrapIndex(p.Selected+delta, len(p.Requests))
}

func (p Permissions) SelectedRequest() PermissionRequest {
	if len(p.Requests) == 0 {
		return PermissionRequest{}
	}
	return p.Requests[clamp(p.Selected, 0, len(p.Requests)-1)]
}
