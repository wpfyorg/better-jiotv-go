package handlers

import (
	"errors"
	"sort"
	"strings"
	"time"

	"github.com/gofiber/fiber/v2"
	"github.com/jiotv-go/jiotv_go/v3/internal/access"
	"github.com/jiotv-go/jiotv_go/v3/internal/config"
	"github.com/jiotv-go/jiotv_go/v3/internal/constants"
	internalUtils "github.com/jiotv-go/jiotv_go/v3/internal/utils"
	"github.com/jiotv-go/jiotv_go/v3/pkg/television"
	"github.com/jiotv-go/jiotv_go/v3/pkg/tvplus"
	"github.com/jiotv-go/jiotv_go/v3/pkg/utils"
)

// The JSON API behind the web interface. Routes under /api/auth/ are open;
// the rest need an admin session (see access.Middleware).

type apiPassword struct {
	Password string `json:"password"`
}

func setSessionCookie(c *fiber.Ctx) error {
	value, err := access.NewSession(time.Now())
	if err != nil {
		return err
	}
	c.Cookie(&fiber.Cookie{
		Name:     access.SessionCookie,
		Value:    value,
		Path:     "/",
		HTTPOnly: true,
		Secure:   c.Protocol() == "https",
		SameSite: fiber.CookieSameSiteStrictMode,
		Expires:  time.Now().Add(access.SessionTTL),
	})
	return nil
}

// APIAuthState reports whether a password is set and whether the request is
// signed in.
func APIAuthState(c *fiber.Ctx) error {
	return c.JSON(fiber.Map{
		"passwordSet":   access.HasPassword(),
		"authenticated": config.Cfg.DisableAuth || access.RequestHasSession(c),
		"keyPresented":  access.Prefix(c) != "",
	})
}

// APISetup sets the first admin password. It needs the access key in the URL
// (/k/<key>/api/auth/setup), so only someone who can read the key can claim
// the server.
func APISetup(c *fiber.Ctx) error {
	if access.HasPassword() {
		return fiber.NewError(fiber.StatusConflict, "a password is already set")
	}
	if access.Prefix(c) == "" && !config.Cfg.DisableAuth {
		return fiber.NewError(fiber.StatusForbidden, "open the setup link that contains the access key")
	}
	body := new(apiPassword)
	if err := c.BodyParser(body); err != nil {
		return internalUtils.BadRequestError(c, "Invalid JSON")
	}
	if err := access.SetPassword(body.Password); err != nil {
		if errors.Is(err, access.ErrWeakPassword) {
			return internalUtils.BadRequestError(c, err.Error())
		}
		return internalUtils.InternalServerError(c, "cannot save the password")
	}
	if err := setSessionCookie(c); err != nil {
		return internalUtils.InternalServerError(c, "cannot start a session")
	}
	return c.JSON(fiber.Map{"status": true})
}

// APILogin signs in with the admin password.
func APILogin(c *fiber.Ctx) error {
	body := new(apiPassword)
	if err := c.BodyParser(body); err != nil {
		return internalUtils.BadRequestError(c, "Invalid JSON")
	}
	ok, err := access.Login(c.IP(), body.Password, time.Now())
	if errors.Is(err, access.ErrTooManyAttempts) {
		return fiber.NewError(fiber.StatusTooManyRequests, err.Error())
	}
	if err != nil {
		return internalUtils.BadRequestError(c, "no password is set yet")
	}
	if !ok {
		return fiber.NewError(fiber.StatusUnauthorized, "wrong password")
	}
	if err := setSessionCookie(c); err != nil {
		return internalUtils.InternalServerError(c, "cannot start a session")
	}
	return c.JSON(fiber.Map{"status": true})
}

// APILogout ends the browser's session.
func APILogout(c *fiber.Ctx) error {
	c.ClearCookie(access.SessionCookie)
	return c.JSON(fiber.Map{"status": true})
}

// APIChangePassword replaces the admin password and signs out other sessions.
func APIChangePassword(c *fiber.Ctx) error {
	body := new(struct {
		Current string `json:"current"`
		New     string `json:"new"`
	})
	if err := c.BodyParser(body); err != nil {
		return internalUtils.BadRequestError(c, "Invalid JSON")
	}
	ok, err := access.Login(c.IP(), body.Current, time.Now())
	if errors.Is(err, access.ErrTooManyAttempts) {
		return fiber.NewError(fiber.StatusTooManyRequests, err.Error())
	}
	if err != nil || !ok {
		return fiber.NewError(fiber.StatusUnauthorized, "the current password is wrong")
	}
	if err := access.SetPassword(body.New); err != nil {
		return internalUtils.BadRequestError(c, err.Error())
	}
	if err := setSessionCookie(c); err != nil {
		return internalUtils.InternalServerError(c, "cannot start a session")
	}
	return c.JSON(fiber.Map{"status": true})
}

// APIStatus reports the logins and the playlist path.
func APIStatus(c *fiber.Ctx) error {
	playlist := ""
	if !config.Cfg.DisableAuth {
		p, err := access.PlaylistPath()
		if err != nil {
			return internalUtils.InternalServerError(c, "cannot read the access key")
		}
		playlist = p
	} else {
		playlist = "/playlist.m3u"
	}
	tvPlus.mu.RLock()
	tvPlusEnabled := tvPlus.client != nil
	tvPlus.mu.RUnlock()
	return c.JSON(fiber.Map{
		"version":        constants.Version,
		"jiotv":          fiber.Map{"loggedIn": jiotvLoggedIn()},
		"tvplus":         fiber.Map{"enabled": tvPlusEnabled, "connected": tvPlusConnected()},
		"playlistPath":   playlist,
		"epgPath":        strings.TrimSuffix(playlist, "playlist.m3u") + "epg.xml.gz",
		"epg":            config.Cfg.EPG,
		"drm":            EnableDRM,
		"logoutDisabled": isLogoutDisabled,
	})
}

type apiChannel struct {
	ID       string `json:"id"`
	Name     string `json:"name"`
	Logo     string `json:"logo"`
	Category string `json:"category"`
	Language string `json:"language"`
	HD       bool   `json:"hd"`
	TVPlus   bool   `json:"tvplus"`
	Catchup  bool   `json:"catchup"`
	// Playable is false for JioTV channels that need a JioTV login which is
	// missing and that TV+ does not carry.
	Playable bool `json:"playable"`
}

// APIChannels lists every channel with its category and language names.
func APIChannels(c *fiber.Ctx) error {
	list, err := television.Channels()
	if err != nil {
		return internalUtils.InternalServerError(c, err.Error())
	}
	all := withTVPlusChannels(list.Result)
	loggedIn := jiotvLoggedIn()
	out := make([]apiChannel, 0, len(all))
	for _, ch := range all {
		logo := ch.LogoURL
		if !strings.HasPrefix(logo, "http://") && !strings.HasPrefix(logo, "https://") {
			logo = "/jtvimage/" + logo
		}
		_, viaTVPlus := tvPlusRoute(ch.ID)
		out = append(out, apiChannel{
			ID:       ch.ID,
			Name:     ch.Name,
			Logo:     logo,
			Category: television.CategoryMap[ch.Category],
			Language: television.LanguageMap[ch.Language],
			HD:       ch.IsHD,
			TVPlus:   strings.HasPrefix(ch.ID, tvplus.IDPrefix),
			Catchup:  ch.IsCatchupAvailable,
			Playable: loggedIn || viaTVPlus || isCustomChannel(ch.ID),
		})
	}
	sort.SliceStable(out, func(i, j int) bool { return out[i].Playable && !out[j].Playable })
	return c.JSON(fiber.Map{"channels": out})
}

// APIJioTVLogout removes the JioTV login.
func APIJioTVLogout(c *fiber.Ctx) error {
	if isLogoutDisabled {
		return fiber.NewError(fiber.StatusForbidden, "logout is disabled")
	}
	if err := utils.Logout(); err != nil {
		return internalUtils.InternalServerError(c, "cannot log out")
	}
	Init()
	return c.JSON(fiber.Map{"status": true})
}

// APITVPlusLogout removes the JioTV+ login.
func APITVPlusLogout(c *fiber.Ctx) error {
	if isLogoutDisabled {
		return fiber.NewError(fiber.StatusForbidden, "logout is disabled")
	}
	if err := tvplus.DeleteCredentials(); err != nil {
		return internalUtils.InternalServerError(c, "cannot log out")
	}
	InitTVPlus()
	return c.JSON(fiber.Map{"status": true})
}

// APIRotateKey replaces the access key. Existing playlist URLs stop working.
func APIRotateKey(c *fiber.Ctx) error {
	if config.Cfg.DisableAuth {
		return internalUtils.BadRequestError(c, "auth is disabled")
	}
	if _, err := access.Rotate(); err != nil {
		return internalUtils.InternalServerError(c, "cannot rotate the key")
	}
	return APIStatus(c)
}
