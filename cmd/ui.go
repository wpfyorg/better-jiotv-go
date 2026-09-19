//go:build !headless

package cmd

import (
	"io/fs"
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

// registerUI adds the web interface: the Svelte app at /, its JSON API
// under /api, and the player pages it embeds.
func registerUI(app *fiber.App) {
	access.SessionCheck = access.RequestHasSession

	ui := web.GetUIFiles()
	app.Use("/ui", filesystem.New(filesystem.Config{Root: http.FS(ui), MaxAge: 31536000}))
	app.Get("/", func(c *fiber.Ctx) error {
		index, err := fs.ReadFile(ui, "index.html")
		if err != nil {
			return err
		}
		c.Set(fiber.HeaderCacheControl, "no-cache")
		c.Type("html")
		return c.Send(index)
	})

	app.Get("/api/auth/state", handlers.APIAuthState)
	app.Post("/api/auth/setup", handlers.APISetup)
	app.Post("/api/auth/login", handlers.APILogin)
	app.Post("/api/auth/logout", handlers.APILogout)
	app.Post("/api/account/password", handlers.APIChangePassword)
	app.Get("/api/status", handlers.APIStatus)
	app.Get("/api/channels", handlers.APIChannels)
	app.Post("/api/jiotv/logout", handlers.APIJioTVLogout)
	app.Post("/api/tvplus/logout", handlers.APITVPlusLogout)
	app.Post("/api/key/rotate", handlers.APIRotateKey)
	app.Get("/api/ott/search", handlers.APIOTTSearch)
	app.Get("/api/ott/screen/:id", handlers.APIOTTScreen)
	app.Get("/api/ott/show/:id", handlers.APIOTTEpisodes)
	app.Get("/api/ott/play/:id", handlers.APIOTTPlay)
	app.Post("/api/ott/license/:id", handlers.APIOTTLicense)

	app.Use("/static", filesystem.New(filesystem.Config{
		Root:       http.FS(web.GetStaticFiles()),
		PathPrefix: "static",
		Browse:     false,
	}))

	app.Get("/classic", handlers.IndexHandler)
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
