// Permission oracle for the isolated pinned-backend proof, using production services/middleware.
// This file is copied beside the proof entry point; it is never part of the backend distribution.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http/httptest"

	"github.com/labstack/echo/v4"
	"papo/internal/middleware"
	"papo/internal/models"
	"papo/internal/services"
	"papo/internal/storage"
)

type permissionCase struct {
	Server     models.ServerSummary  `json:"server"`
	Channel    models.ChannelSummary `json:"channel"`
	Roles      []models.Role         `json:"roles"`
	Member     models.UserSummary    `json:"member"`
	Global     []bool                `json:"global"`
	Operations []bool                `json:"operations,omitempty"`
}

func registerPermissionProof(e *echo.Echo) {
	e.POST("/__proof/permissions", func(c echo.Context) error {
		cases, err := permissionCases(c.Request().Context(), e)
		if err != nil {
			return c.NoContent(500)
		}
		direct, err := directPermissionCases(c.Request().Context())
		if err != nil {
			return c.NoContent(500)
		}
		return c.JSON(200, map[string]any{"ordinary": cases, "direct": direct})
	})
}

func permissionCases(ctx context.Context, e *echo.Echo) ([]permissionCase, error) {
	owner, _, err := storage.CreateUser(ctx, "permission-owner", "unused", "")
	if err != nil {
		return nil, err
	}
	member, _, err := storage.CreateUser(ctx, "permission-member", "unused", "")
	if err != nil {
		return nil, err
	}
	other, _, err := storage.CreateUser(ctx, "permission-other", "unused", "")
	if err != nil {
		return nil, err
	}
	old, err := storage.GetServer(ctx)
	if err != nil {
		return nil, err
	}
	// Restore the proof's normal owner before the existing live example runs.
	defer storage.GetDB().ExecContext(context.Background(), "UPDATE servers SET owner_id=$1", old.OwnerID)
	if _, err := storage.GetDB().ExecContext(ctx, "UPDATE servers SET owner_id=$1", owner.ID); err != nil {
		return nil, err
	}
	a, err := storage.CreateRole(ctx, "permission-a", nil, models.RolePermissions{})
	if err != nil {
		return nil, err
	}
	b, err := storage.CreateRole(ctx, "permission-b", nil, models.RolePermissions{})
	if err != nil {
		return nil, err
	}
	channel, err := storage.CreateChannel(ctx, "permission-proof", "voice", "")
	if err != nil {
		return nil, err
	}
	var cases []permissionCase
	// Every combination of the two explicit roles, including no role, across all pairs of
	// four-bit channel overwrites. Expected decisions come from production operations below.
	for assignments := 0; assignments < 4; assignments++ {
		for first := 0; first < 16; first++ {
			for second := 0; second < 16; second++ {
				if err := assignProofRoles(ctx, member.ID, a.ID, b.ID, assignments); err != nil {
					return nil, err
				}
				overrides := map[string]models.ChannelPermission{a.ID: channelBits(first), b.ID: channelBits(second)}
				permissions, err := json.Marshal(overrides)
				if err != nil {
					return nil, err
				}
				if _, err := storage.GetDB().ExecContext(ctx, "UPDATE channels SET permissions=$1 WHERE id=$2", permissions, channel.ID); err != nil {
					return nil, err
				}
				if err := setProofRole(ctx, a.ID, first*17); err != nil {
					return nil, err
				}
				if err := setProofRole(ctx, b.ID, second*17); err != nil {
					return nil, err
				}
				result, err := capturePermissionCase(ctx, e, channel.ID, member.ID, owner.ID, true)
				if err != nil {
					return nil, err
				}
				cases = append(cases, result)
			}
		}
	}
	// All eight global bits, split across roles, for each explicit assignment combination.
	for bits := 0; bits < 256; bits++ {
		for assignments := 0; assignments < 4; assignments++ {
			if err := assignProofRoles(ctx, member.ID, a.ID, b.ID, assignments); err != nil {
				return nil, err
			}
			if err := setProofRole(ctx, a.ID, bits&0x55); err != nil {
				return nil, err
			}
			if err := setProofRole(ctx, b.ID, bits&0xaa); err != nil {
				return nil, err
			}
			result, err := capturePermissionCase(ctx, e, channel.ID, member.ID, owner.ID, false)
			if err != nil {
				return nil, err
			}
			cases = append(cases, result)
		}
	}
	// Open-channel defaults, owner override, and each channel type.
	for _, kind := range []string{"text", "voice", "category"} {
		for _, open := range []bool{false, true} {
			permissions := []byte(`{}`)
			if !open {
				permissions, err = json.Marshal(map[string]models.ChannelPermission{a.ID: {}})
				if err != nil {
					return nil, err
				}
			}
			if _, err := storage.GetDB().ExecContext(ctx, "UPDATE channels SET permissions=$1,type=$2 WHERE id=$3", permissions, kind, channel.ID); err != nil {
				return nil, err
			}
			for _, user := range []string{owner.ID, other.ID} {
				result, err := capturePermissionCase(ctx, e, channel.ID, user, owner.ID, true)
				if err != nil {
					return nil, err
				}
				cases = append(cases, result)
			}
		}
	}
	return cases, nil
}

func assignProofRoles(ctx context.Context, user, a, b string, bits int) error {
	if _, err := storage.GetDB().ExecContext(ctx, "DELETE FROM user_roles WHERE user_id=$1", user); err != nil {
		return err
	}
	for index, role := range []string{a, b} {
		if bits&(1<<index) != 0 {
			if _, err := storage.AssignUserRole(ctx, user, role); err != nil {
				return err
			}
		}
	}
	return nil
}
func setProofRole(ctx context.Context, id string, bits int) error {
	value, err := json.Marshal(models.RolePermissions{ManageServer: bits&1 != 0, ManageChannels: bits&2 != 0, ManageRoles: bits&4 != 0, ManageWebhooks: bits&8 != 0, BanMembers: bits&16 != 0, PinMessage: bits&32 != 0, EveryoneMessage: bits&64 != 0, SendAttachment: bits&128 != 0})
	if err != nil {
		return err
	}
	_, err = storage.GetDB().ExecContext(ctx, "UPDATE roles SET permissions=$1 WHERE id=$2", value, id)
	return err
}
func channelBits(bits int) models.ChannelPermission {
	return models.ChannelPermission{ReadChannel: bits&1 != 0, SendMessages: bits&2 != 0, DeleteMessages: bits&4 != 0, ConnectVoice: bits&8 != 0}
}
func globalProof(e *echo.Echo, user string) ([]bool, error) {
	selectors := []func(models.RolePermissions) bool{
		func(p models.RolePermissions) bool { return p.ManageServer }, func(p models.RolePermissions) bool { return p.ManageChannels },
		func(p models.RolePermissions) bool { return p.ManageRoles }, func(p models.RolePermissions) bool { return p.ManageWebhooks },
		func(p models.RolePermissions) bool { return p.BanMembers }, func(p models.RolePermissions) bool { return p.PinMessage },
		func(p models.RolePermissions) bool { return p.EveryoneMessage }, func(p models.RolePermissions) bool { return p.SendAttachment },
	}
	result := make([]bool, 0, len(selectors))
	for _, selector := range selectors {
		recorder := httptest.NewRecorder()
		c := e.NewContext(httptest.NewRequest("GET", "/", nil), recorder)
		c.Set(middleware.UserIDContextKey, user)
		accepted := false
		err := middleware.RequireRolePermission(selector)(func(echo.Context) error { accepted = true; return nil })(c)
		if err != nil {
			return nil, err
		}
		if !accepted && recorder.Code != 403 {
			return nil, fmt.Errorf("unexpected middleware outcome")
		}
		result = append(result, accepted)
	}
	return result, nil
}
func proofMessage(ctx context.Context, channel, author string) (string, error) {
	var id string
	err := storage.GetDB().QueryRowContext(ctx, "INSERT INTO messages (channel_id,author_id) VALUES ($1,$2) RETURNING id", channel, author).Scan(&id)
	return id, err
}
func capturePermissionCase(ctx context.Context, e *echo.Echo, channel, user, owner string, operations bool) (permissionCase, error) {
	var result permissionCase
	var err error
	result.Server, err = services.GetServer(ctx)
	if err != nil {
		return result, err
	}
	channels, err := storage.ListChannelSummaries(ctx, user)
	if err != nil {
		return result, err
	}
	for _, item := range channels {
		if item.ID == channel {
			result.Channel = item
		}
	}
	if result.Channel.ID == "" {
		return result, fmt.Errorf("channel snapshot missing")
	}
	result.Roles, err = storage.ListRoles(ctx)
	if err != nil {
		return result, err
	}
	members, err := services.ListUsers(ctx, nil, "")
	if err != nil {
		return result, err
	}
	for _, item := range members.Users {
		if item.ID == user {
			result.Member = item
		}
	}
	result.Global, err = globalProof(e, user)
	if err != nil {
		return result, err
	}
	if !operations {
		return result, nil
	}
	result.Operations, err = operationProof(ctx, channel, user, owner)
	return result, err
}

func operationProof(ctx context.Context, channel, user, owner string) ([]bool, error) {
	readError := services.CanReadChannel(ctx, channel, user)
	voiceError := services.CanConnectVoice(ctx, channel, user)
	sent, sendError := services.CreateMessage(ctx, channel, user, user, "", nil, nil)
	if sendError == nil {
		defer storage.GetDB().ExecContext(context.Background(), "DELETE FROM messages WHERE id=$1", sent.ID)
	}
	id, err := proofMessage(ctx, channel, owner)
	if err != nil {
		return nil, err
	}
	defer storage.GetDB().ExecContext(context.Background(), "DELETE FROM messages WHERE id=$1", id)
	_, _, pinError := services.PinMessage(ctx, channel, id, user)
	if _, err := storage.GetDB().ExecContext(ctx, "INSERT INTO pinned_messages (channel_id,message_id,pinned_by) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING", channel, id, owner); err != nil {
		return nil, err
	}
	_, unpinError := services.UnpinMessage(ctx, channel, id, user)
	_, deleteError := services.DeleteMessage(ctx, id, user)
	ownID, err := proofMessage(ctx, channel, user)
	if err != nil {
		return nil, err
	}
	_, editError := services.EditMessage(ctx, ownID, user, user, nil)
	_, ownDeleteError := services.DeleteMessage(ctx, ownID, user)
	for _, err := range []error{readError, voiceError, sendError, pinError, unpinError, deleteError, editError, ownDeleteError} {
		if err != nil && !errors.Is(err, services.ErrPermissionDenied) && !errors.Is(err, services.ErrDirectMessageBlocked) && !errors.Is(err, services.ErrInvalidChannelType) {
			return nil, err
		}
	}
	return []bool{readError == nil, sendError == nil, voiceError == nil, pinError == nil, unpinError == nil, deleteError == nil, editError == nil, ownDeleteError == nil}, nil
}

type directCase struct {
	Member     bool   `json:"member"`
	Blocked    bool   `json:"blocked"`
	Operations []bool `json:"operations"`
}

func directPermissionCases(ctx context.Context) ([]directCase, error) {
	users, err := services.ListUsers(ctx, nil, "")
	if err != nil {
		return nil, err
	}
	ids := map[string]string{}
	for _, user := range users.Users {
		ids[user.Username] = user.ID
	}
	owner, member, other := ids["permission-owner"], ids["permission-member"], ids["permission-other"]
	old, err := storage.GetServer(ctx)
	if err != nil {
		return nil, err
	}
	defer storage.GetDB().ExecContext(context.Background(), "UPDATE servers SET owner_id=$1", old.OwnerID)
	if _, err := storage.GetDB().ExecContext(ctx, "UPDATE servers SET owner_id=$1", owner); err != nil {
		return nil, err
	}
	dm, _, err := services.OpenDirectConversation(ctx, member, other)
	if err != nil {
		return nil, err
	}
	var cases []directCase
	for state := 0; state < 4; state++ {
		if _, err := storage.GetDB().ExecContext(ctx, "DELETE FROM user_blocks WHERE user_id IN ($1,$2)", member, other); err != nil {
			return nil, err
		}
		if state == 1 {
			if err := storage.CreateUserBlock(ctx, member, other); err != nil {
				return nil, err
			}
		}
		if state == 2 {
			if err := storage.CreateUserBlock(ctx, other, member); err != nil {
				return nil, err
			}
		}
		if state == 3 {
			if err := services.HideDirectConversation(ctx, member, dm.ID); err != nil {
				return nil, err
			}
		}
		for _, user := range []string{owner, member, other} {
			membership, blocked, err := storage.DirectConversationAccess(ctx, dm.ID, user)
			if err != nil {
				return nil, err
			}
			author := member
			if user == member {
				author = other
			}
			operations, err := operationProof(ctx, dm.ID, user, author)
			if err != nil {
				return nil, err
			}
			cases = append(cases, directCase{Member: membership, Blocked: blocked, Operations: operations})
		}
	}
	if _, _, err := services.OpenDirectConversation(ctx, member, member); err == nil {
		return nil, fmt.Errorf("self DM accepted")
	}
	return cases, nil
}
