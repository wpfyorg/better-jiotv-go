// Package access gates JioTV Go behind an access key.
//
// IPTV players cannot log in, so their URLs carry the key as a path prefix:
// /k/<key>/playlist.m3u. The gate strips the prefix before routing and keeps
// it, so handlers can put it into the URLs they generate.
//
// Stream proxy routes (/render.*, /drm) are left open: their parameters are
// encrypted with a key that is regenerated on every start, so they cannot be
// forged and are only handed out through gated routes.
package access

import (
	"crypto/rand"
	"crypto/subtle"
	"encoding/hex"
	"errors"
	"strings"
	"sync"

	"github.com/gofiber/fiber/v2"
	"github.com/jiotv-go/jiotv_go/v3/pkg/store"
)

const (
	storeKey  = "access_key"
	prefix    = "/k/"
	localsKey = "access_prefix"
)

// SessionCheck reports whether a request carries a valid UI session. The
// full build sets it; the headless build has no sessions.
var SessionCheck func(c *fiber.Ctx) bool

var (
	mu  sync.Mutex
	key string
)

// Key returns the access key, creating and storing one on first use.
func Key() (string, error) {
	mu.Lock()
	defer mu.Unlock()
	if key != "" {
		return key, nil
	}
	stored, err := store.Get(storeKey)
	if err == nil && stored != "" {
		key = stored
		return key, nil
	}
	return rotateLocked()
}

// Rotate replaces the access key. Old playlist URLs stop working.
func Rotate() (string, error) {
	mu.Lock()
	defer mu.Unlock()
	return rotateLocked()
}

func rotateLocked() (string, error) {
	b := make([]byte, 16)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	next := hex.EncodeToString(b)
	if err := store.Set(storeKey, next); err != nil {
		return "", err
	}
	key = next
	return key, nil
}

// PlaylistPath is the gated playlist path for the current key.
func PlaylistPath() (string, error) {
	k, err := Key()
	if err != nil {
		return "", err
	}
	return prefix + k + "/playlist.m3u", nil
}

// openPaths need no key.
var openPaths = []string{
	"/render.m3u8", "/render.ts", "/render.key", "/render.mpd", "/render.dash",
	"/drm", "/dashtime", "/static/", "/favicon.ico", "/jtvimage/", "/jtvposter/",
	"/api/auth/",
}

func isOpen(path string) bool {
	for _, p := range openPaths {
		if path == p || strings.HasPrefix(path, p) {
			return true
		}
	}
	return false
}

var errBadKey = errors.New("invalid access key")

// Middleware lets a request through when its path starts with /k/<key>/, when
// the route is open, or when SessionCheck accepts it. Otherwise it answers 401.
func Middleware() fiber.Handler {
	return func(c *fiber.Ctx) error {
		path := c.Path()
		if strings.HasPrefix(path, prefix) {
			rest := strings.TrimPrefix(path, prefix)
			given, tail, _ := strings.Cut(rest, "/")
			want, err := Key()
			if err != nil {
				return fiber.NewError(fiber.StatusInternalServerError, "access key unavailable")
			}
			if subtle.ConstantTimeCompare([]byte(given), []byte(want)) != 1 {
				return fiber.NewError(fiber.StatusUnauthorized, errBadKey.Error())
			}
			c.Locals(localsKey, prefix+given)
			c.Path("/" + tail)
			return c.Next()
		}
		if isOpen(path) {
			return c.Next()
		}
		if SessionCheck != nil && SessionCheck(c) {
			return c.Next()
		}
		return fiber.NewError(fiber.StatusUnauthorized, "unauthorized")
	}
}

// Prefix returns the /k/<key> prefix the request came in with, or "".
func Prefix(c *fiber.Ctx) string {
	p, _ := c.Locals(localsKey).(string)
	return p
}

// BaseURL returns scheme://host plus the key prefix of the request, for
// building URLs that an IPTV player will request later.
func BaseURL(c *fiber.Ctx) string {
	return strings.ToLower(c.Protocol()) + "://" + c.Hostname() + Prefix(c)
}
