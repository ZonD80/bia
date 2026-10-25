package main

import (
	"context"
	"errors"
	"fmt"
	"sort"
	"strconv"
	"strings"
	"time"

	"go.mau.fi/whatsmeow"
	waTypes "go.mau.fi/whatsmeow/types"

	"go.mau.fi/mautrix-meta/pkg/instameow/slidetypes"
	"go.mau.fi/mautrix-meta/pkg/messagix/socket"
)

// Group administration. Messenger's encrypted groups run on WhatsApp's protocol and have
// its settings; classic Messenger groups and Instagram groups only have admins (plus the
// name and photo, which have their own methods).

var errClassicSettings = errors.New("classic (unencrypted) Messenger groups have no such settings: only admins, the name and the photo can be changed (Facebook moves groups to encryption over time)")

func secsText(s int) string {
	switch {
	case s <= 0:
		return "off"
	case s%86400 == 0:
		return fmt.Sprintf("%d day(s)", s/86400)
	case s%3600 == 0:
		return fmt.Sprintf("%d hour(s)", s/3600)
	}
	return fmt.Sprintf("%ds", s)
}

// --- Messenger ---

func (m *messenger) groupTarget(ctx context.Context, chat string) (*target, error) {
	if _, err := m.client(); errors.Is(err, ErrNotLoggedIn) {
		return nil, err
	}
	t, err := m.target(ctx, chat)
	if err != nil {
		return nil, err
	}
	if !t.group {
		return nil, fmt.Errorf("%s is not a group", t.key)
	}
	return t, nil
}

func (m *messenger) GroupInfo(ctx context.Context, p ChatParams) (map[string]any, error) {
	t, err := m.groupTarget(ctx, p.Chat)
	if err != nil {
		return nil, err
	}
	if !t.e2ee {
		info, err := m.Members(ctx, p)
		if err != nil {
			return nil, err
		}
		info["kind"] = "classic group (unencrypted)"
		info["note"] = errClassicSettings.Error()
		return info, nil
	}
	e2ee, err := m.encrypted(ctx)
	if err != nil {
		return nil, err
	}
	gi, err := e2ee.GetGroupInfo(ctx, t.jid)
	if err != nil {
		return nil, fmt.Errorf("group info: %w", err)
	}
	own := m.ownID()
	var members []map[string]any
	imAdmin := false
	for _, pc := range gi.Participants {
		id := pc.JID.User
		if id == own && (pc.IsAdmin || pc.IsSuperAdmin) {
			imAdmin = true
		}
		role := "member"
		if pc.IsSuperAdmin {
			role = "owner"
		} else if pc.IsAdmin {
			role = "admin"
		}
		members = append(members, map[string]any{"id": id, "name": m.st.UserName(ctx, id), "role": role, "admin": pc.IsAdmin || pc.IsSuperAdmin})
	}
	sort.Slice(members, func(i, j int) bool { return fmt.Sprint(members[i]["name"]) < fmt.Sprint(members[j]["name"]) })
	who := func(adminsOnly bool) string {
		if adminsOnly {
			return "admins only"
		}
		return "everyone"
	}
	out := map[string]any{
		"kind": "encrypted group", "title": gi.Name, "description": gi.Topic, "members": members,
		"settings": []string{
			"who may send: " + who(gi.IsAnnounce),
			"who may edit the group info: " + who(gi.IsLocked),
			"who may add members: " + who(gi.MemberAddMode == waTypes.GroupMemberAddModeAdmin),
			"join approval: " + map[bool]string{true: "on", false: "off"}[gi.IsJoinApprovalRequired],
			"disappearing messages: " + secsText(int(gi.DisappearingTimer)),
		},
		"you_admin": imAdmin,
	}
	if gi.IsJoinApprovalRequired {
		if reqs, err := e2ee.GetGroupRequestParticipants(ctx, t.jid); err == nil && len(reqs) > 0 {
			out["pending_requests"] = len(reqs)
		}
	}
	return out, nil
}

func (m *messenger) memberID(ctx context.Context, member string) (int64, error) {
	ids, err := m.userIDs(ctx, []string{member})
	if err != nil {
		return 0, err
	}
	return ids[0], nil
}

func (m *messenger) SetAdmin(ctx context.Context, p AdminParams) error {
	t, err := m.groupTarget(ctx, p.Chat)
	if err != nil {
		return err
	}
	id, err := m.memberID(ctx, p.Member)
	if err != nil {
		return err
	}
	if t.e2ee {
		e2ee, err := m.encrypted(ctx)
		if err != nil {
			return err
		}
		action := whatsmeow.ParticipantChangePromote
		if !p.Admin {
			action = whatsmeow.ParticipantChangeDemote
		}
		res, err := e2ee.UpdateGroupParticipants(ctx, t.jid, []waTypes.JID{waTypes.NewJID(strconv.FormatInt(id, 10), waTypes.MessengerServer)}, action)
		if err != nil {
			return fmt.Errorf("%w (only group admins can do this)", err)
		}
		if len(res) > 0 && res[0].Error != 0 {
			return fmt.Errorf("refused (code %d): only admins can change admins, and the person must be in the group", res[0].Error)
		}
		return nil
	}
	cli, err := m.readyClient(ctx)
	if err != nil {
		return err
	}
	_, err = cli.ExecuteTasks(ctx, &socket.UpdateAdminTask{ThreadKey: t.fbKey, ContactID: id, IsAdmin: b2i(p.Admin)})
	if err == nil {
		m.st.PutKV(ctx, "member:"+t.key+":"+strconv.FormatInt(id, 10), b2s(p.Admin))
	}
	return err
}

func (m *messenger) GroupSettings(ctx context.Context, p GroupSettingsParams) ([]string, error) {
	t, err := m.groupTarget(ctx, p.Chat)
	if err != nil {
		return nil, err
	}
	if !t.e2ee {
		return nil, errClassicSettings
	}
	e2ee, err := m.encrypted(ctx)
	if err != nil {
		return nil, err
	}
	var done []string
	fail := func(what string, err error) ([]string, error) {
		msg := fmt.Sprintf("%s: %v (only group admins can change it)", what, err)
		if len(done) > 0 {
			msg += "; already changed: " + strings.Join(done, "; ")
		}
		return nil, errors.New(msg)
	}
	who := func(on bool) string {
		if on {
			return "only admins"
		}
		return "everyone"
	}
	if v := p.OnlyAdminsSend; v != nil {
		if err := e2ee.SetGroupAnnounce(ctx, t.jid, *v); err != nil {
			return fail("who may send", err)
		}
		done = append(done, who(*v)+" may send")
	}
	if v := p.OnlyAdminsEditInfo; v != nil {
		if err := e2ee.SetGroupLocked(ctx, t.jid, *v); err != nil {
			return fail("who may edit info", err)
		}
		done = append(done, who(*v)+" may edit the group info")
	}
	if v := p.OnlyAdminsAdd; v != nil {
		mode := waTypes.GroupMemberAddModeAllMember
		if *v {
			mode = waTypes.GroupMemberAddModeAdmin
		}
		if err := e2ee.SetGroupMemberAddMode(ctx, t.jid, mode); err != nil {
			return fail("who may add members", err)
		}
		done = append(done, who(*v)+" may add members")
	}
	if v := p.JoinApproval; v != nil {
		if err := e2ee.SetGroupJoinApprovalMode(ctx, t.jid, *v); err != nil {
			return fail("join approval", err)
		}
		done = append(done, "join approval "+map[bool]string{true: "on", false: "off"}[*v])
	}
	if v := p.DisappearingSecs; v != nil {
		if err := e2ee.SetDisappearingTimer(ctx, t.jid, time.Duration(max(*v, 0))*time.Second, time.Now()); err != nil {
			return fail("disappearing messages", err)
		}
		done = append(done, "disappearing messages "+secsText(*v))
	}
	if v := p.Description; v != nil {
		if err := e2ee.SetGroupDescription(ctx, t.jid, strings.TrimSpace(*v)); err != nil {
			return fail("description", err)
		}
		done = append(done, "description set")
	}
	if len(done) == 0 {
		return nil, errors.New("nothing to change: pass one of the settings")
	}
	return done, nil
}

func (m *messenger) JoinRequests(ctx context.Context, p ChatParams) ([]map[string]any, error) {
	t, err := m.groupTarget(ctx, p.Chat)
	if err != nil {
		return nil, err
	}
	if !t.e2ee {
		return nil, errClassicSettings
	}
	e2ee, err := m.encrypted(ctx)
	if err != nil {
		return nil, err
	}
	reqs, err := e2ee.GetGroupRequestParticipants(ctx, t.jid)
	if err != nil {
		return nil, fmt.Errorf("%w (only admins see join requests)", err)
	}
	out := make([]map[string]any, 0, len(reqs))
	for _, r := range reqs {
		out = append(out, map[string]any{"id": r.JID.User, "name": m.st.UserName(ctx, r.JID.User), "ts": r.RequestedAt.Unix()})
	}
	return out, nil
}

func (m *messenger) AnswerJoin(ctx context.Context, p AnswerJoinParams) error {
	t, err := m.groupTarget(ctx, p.Chat)
	if err != nil {
		return err
	}
	if !t.e2ee {
		return errClassicSettings
	}
	e2ee, err := m.encrypted(ctx)
	if err != nil {
		return err
	}
	id := strings.TrimSpace(p.Member)
	if !isDigits(id) {
		n, err := m.memberID(ctx, id)
		if err != nil {
			return err
		}
		id = strconv.FormatInt(n, 10)
	}
	action := whatsmeow.ParticipantChangeApprove
	if !p.Approve {
		action = whatsmeow.ParticipantChangeReject
	}
	res, err := e2ee.UpdateGroupRequestParticipants(ctx, t.jid, []waTypes.JID{waTypes.NewJID(id, waTypes.MessengerServer)}, action)
	if err != nil {
		return err
	}
	if len(res) > 0 && res[0].Error != 0 {
		return fmt.Errorf("refused (code %d): is there a pending request from them? (join_requests lists them)", res[0].Error)
	}
	return nil
}

// --- Instagram ---

var errIGSettings = errors.New("Instagram groups have no such settings: only admins, members, the name and the photo can be changed")

func (ig *instagram) GroupInfo(ctx context.Context, p ChatParams) (map[string]any, error) {
	info, err := ig.Members(ctx, p)
	if err != nil {
		return nil, err
	}
	info["kind"] = "group"
	return info, nil
}

func (ig *instagram) SetAdmin(ctx context.Context, p AdminParams) error {
	cli, err := ig.readyClient(ctx)
	if err != nil {
		return err
	}
	t, err := ig.target(ctx, p.Chat)
	if err != nil {
		return err
	}
	if !t.group {
		return errors.New("not a group")
	}
	users, err := ig.people(ctx, []string{p.Member})
	if err != nil {
		return err
	}
	if users[0].IGID == "" {
		return errors.New("no Instagram id for that person")
	}
	req := &slidetypes.ModifyAdminsRequest{ThreadID: t.id, UserIGIDs: []string{users[0].IGID}}
	if p.Admin {
		_, err = cli.AddAdmins(ctx, req)
	} else {
		_, err = cli.RemoveAdmins(ctx, req)
	}
	if err != nil {
		return fmt.Errorf("%w (only group admins can change admins)", err)
	}
	ig.st.PutKV(ctx, "member:"+t.id+":"+users[0].ID, b2s(p.Admin))
	return nil
}

func (ig *instagram) GroupSettings(ctx context.Context, p GroupSettingsParams) ([]string, error) {
	return nil, errIGSettings
}

func (ig *instagram) JoinRequests(ctx context.Context, p ChatParams) ([]map[string]any, error) {
	return nil, errors.New("Instagram groups have no join requests (people are added by members)")
}

func (ig *instagram) AnswerJoin(ctx context.Context, p AnswerJoinParams) error {
	return errors.New("Instagram groups have no join requests (people are added by members)")
}
