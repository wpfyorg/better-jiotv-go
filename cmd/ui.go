//go:build !headless

package cmd

import (
	"crypto/subtle"
	"net/http"
	"strings"

	"github.com/gofiber/fiber/v2"
	"github.com/gofiber/fiber/v2/middleware/filesystem"
	"github.com/gofiber/template/html/v2"
	"github.com/jiotv-go/jiotv_go/v3/internal/access"
	"github.com/jiotv-go/jiotv_go/v3/internal/config"
	"github.com/jiotv-go/jiotv_go/v3/internal/handlers"
	"github.com/jiotv-go/jiotv_go/v3/pkg/tvplus"
	"github.com/jiotv-go/jiotv_go/v3/web"
)

// Headless reports whether this is the headless build.
const Headless = false

func newViews() fiber.Views {
	engine := html.NewFileSystem(http.FS(web.GetViewFiles()), ".html")
	if config.Cfg.Debug {
		engine.Reload(true)
	}
	engine.AddFunc("isTVPlus", func(channelID string) bool {
		return strings.HasPrefix(channelID, tvplus.IDPrefix)
	})
	return engine
}

// keyCookie lets a browser that opened /k/<key>/ use the web interface.
const keyCookie = "jiotv_key"

// registerUI adds the web interface and its player.
func registerUI(app *fiber.App) {
	access.SessionCheck = func(c *fiber.Ctx) bool {
		want, err := access.Key()
		got := c.Cookies(keyCookie)
		return err == nil && got != "" && subtle.ConstantTimeCompare([]byte(got), []byte(want)) == 1
	}
	app.Use(func(c *fiber.Ctx) error {
		if prefix := access.Prefix(c); prefix != "" && c.Method() == fiber.MethodGet {
			c.Cookie(&fiber.Cookie{
				Name:     keyCookie,
				Value:    strings.TrimPrefix(prefix, "/k/"),
				Path:     "/",
				HTTPOnly: true,
				SameSite: fiber.CookieSameSiteLaxMode,
				MaxAge:   365 * 24 * 3600,
			})
		}
		return c.Next()
	})

	app.Use("/static", filesystem.New(filesystem.Config{
		Root:       http.FS(web.GetStaticFiles()),
		PathPrefix: "static",
		Browse:     false,
	}))

	app.Get("/", handlers.IndexHandler)
	app.Post("/login/sendOTP", handlers.LoginSendOTPHandler)
	app.Post("/login/verifyOTP", handlers.LoginVerifyOTPHandler)
	app.Get("/logout", handlers.LogoutHandler)
	app.Post("/tvplus/login/sendOTP", handlers.TVPlusSendOTPHandler)
	app.Post("/tvplus/login/verifyOTP", handlers.TVPlusVerifyOTPHandler)
	app.Get("/tvplus/logout", handlers.TVPlusLogoutHandler)
	app.Get("/play/:id", handlers.PlayHandler)
	app.Get("/player/:id", handlers.PlayerHandler)
	app.Get("/premium/providers", handlers.PremiumProvidersHandler)
	app.Get("/premium/providers/:id/catalog", handlers.PremiumProviderCatalogHandler)
	app.Get("/premium/providers/:id/watch", handlers.PremiumProviderWatchHandler)
	app.Get("/premium/providers/:id/play", handlers.PremiumProviderPlayHandler)
	app.Get("/premium/player", handlers.PremiumPlayerHandler)
	app.Get("/catchup/:id", handlers.CatchupHandler)
	app.Get("/catchup/play/:id", handlers.CatchupPlayerHandler)
	app.Get("/catchup/render/:id", handlers.CatchupRenderPlayerHandler)
	app.Get("/favicon.ico", handlers.FaviconHandler)
	app.Get("/epg/:channelID/:offset", handlers.WebEPGHandler)
	app.Get("/jtvposter/:date/:file", handlers.PosterHandler)
	app.Get("/mpd/:channelID", handlers.LiveMpdHandler)
}
