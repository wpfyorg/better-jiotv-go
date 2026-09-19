package cmd

import (
	"fmt"
	"log" // Added import for *log.Logger type
	"strings"
	"time"

	"github.com/jiotv-go/jiotv_go/v3/internal/access"
	"github.com/jiotv-go/jiotv_go/v3/internal/config"
	"github.com/jiotv-go/jiotv_go/v3/internal/constants"
	"github.com/jiotv-go/jiotv_go/v3/internal/constants/tasks"
	"github.com/jiotv-go/jiotv_go/v3/internal/handlers"
	"github.com/jiotv-go/jiotv_go/v3/internal/middleware"
	"github.com/jiotv-go/jiotv_go/v3/pkg/epg"
	"github.com/jiotv-go/jiotv_go/v3/pkg/scheduler"
	"github.com/jiotv-go/jiotv_go/v3/pkg/utils"

	"github.com/gofiber/fiber/v2"
	"github.com/gofiber/fiber/v2/middleware/logger"
	"github.com/gofiber/fiber/v2/middleware/recover"
)

// LoadConfig loads the application configuration from the given path.
func LoadConfig(configPath string) error {
	return config.Cfg.Load(configPath)
}

// InitializeLogger initializes the global logger.
// This should be called after LoadConfig.
func InitializeLogger() {
	utils.Log = utils.GetLogger()
}

// Logger returns the initialized global logger.
// Ensure InitializeLogger has been called before using this.
func Logger() *log.Logger { // Corrected to *log.Logger
	return utils.Log
}

type JioTVServerConfig struct {
	Host        string
	Port        string
	TLS         bool
	TLSCertPath string
	TLSKeyPath  string
}

// JioTVServer starts the JioTV server.
// Assumes config and logger are already initialized.
// It initializes secure URLs, EPG, store, and handlers.
// It then configures the Fiber app with middleware and routes.
// It starts listening on the provided host and port.
// Returns an error if listening fails.
func JioTVServer(jiotvServerConfig JioTVServerConfig) error {
	// Config, Logger and Store are assumed to be initialized in main.go

	// if config EPG is true or file epg.xml.gz exists
	if config.Cfg.TVPlus {
		epg.RegisterSource(handlers.TVPlusEPGSource)
	}

	if config.Cfg.EPG || utils.FileExists("epg.xml.gz") {
		go epg.Init()
	}

	// Start Scheduler
	scheduler.Init()
	defer scheduler.Stop()

	if config.Cfg.TVPlus {
		scheduler.Add(tasks.TVPlusRefreshTokenTaskID, 30*time.Minute, handlers.RefreshTVPlusTokenTask)
	}

	app := fiber.New(fiber.Config{
		Views:             newViews(),
		Network:           fiber.NetworkTCP,
		StreamRequestBody: true,
		CaseSensitive:     false,
		StrictRouting:     false,
		EnablePrintRoutes: false,
		ServerHeader:      "JioTV Go",
		AppName:           fmt.Sprintf("JioTV Go %s", constants.Version),
	})

	app.Use(recover.New(recover.Config{
		EnableStackTrace: true,
	}))

	app.Use(middleware.CORS())

	app.Use(logger.New(logger.Config{
		TimeZone: "Asia/Kolkata",
		Format:   "[${time}] ${status} - ${latency} ${method} ${path} Params:[${queryParams}] ${error}\n",
		Output:   utils.Log.Writer(),
	}))

	if config.Cfg.DisableAuth {
		utils.Log.Println("WARNING: auth is disabled; anyone who can reach this server can use it")
	} else {
		if config.Cfg.DisableURLEncryption {
			// Stream proxy routes are open because their encrypted parameters
			// cannot be forged; without encryption they would be an open proxy.
			return fmt.Errorf("disable_url_encryption needs disable_auth: the access key relies on encrypted stream URLs")
		}
		playlistPath, err := access.PlaylistPath()
		if err != nil {
			return fmt.Errorf("cannot load the access key: %w", err)
		}
		app.Use(access.Middleware())
		fmt.Printf("Playlist: http://%s:%s%s\n", displayHost(jiotvServerConfig.Host), jiotvServerConfig.Port, playlistPath)
		if !Headless && !access.HasPassword() {
			fmt.Printf("Web setup: http://%s:%s%s (or run: jiotv_go admin password)\n", displayHost(jiotvServerConfig.Host), jiotvServerConfig.Port, strings.TrimSuffix(playlistPath, "playlist.m3u"))
		}
	}

	registerUI(app)

	handlers.Init()

	// Routes used by IPTV players. Both builds serve them.
	app.Use("/out/", handlers.SLHandler)
	app.Get("/live/mpd/:channelID", handlers.LiveManifestMpdHandler)
	app.Post("/live/key/:channelID", handlers.LiveManifestKeyHandler)
	app.Get("/live/:id", handlers.LiveHandler)
	app.Get("/live/:quality/:id", handlers.LiveQualityHandler)
	app.Get("/render.m3u8", handlers.RenderHandler)
	app.Get("/render.ts", handlers.RenderTSHandler)
	app.Get("/render.key", handlers.RenderKeyHandler)
	app.Get("/channels", handlers.ChannelsHandler)
	app.Get("/playlist.m3u", handlers.PlaylistHandler)
	app.Get("/catchup/stream/:id", handlers.CatchupStreamHandler)
	app.Get("/jtvimage/:file", handlers.ImageHandler)
	app.Get("/epg.xml.gz", handlers.EPGHandler)
	app.Post("/drm", handlers.DRMKeyHandler)
	app.Get("/dashtime", handlers.DASHTimeHandler)
	app.Get("/render.mpd", handlers.MpdHandler)
	app.Use("/render.dash", handlers.DashHandler)
	app.Get("/vod.m3u", handlers.VODPlaylistHandler)
	app.Get("/vod/:id", handlers.VODStreamHandler)
	app.Post("/vod/license/:id", handlers.APIOTTLicense)

	if jiotvServerConfig.TLS {
		if jiotvServerConfig.TLSCertPath == "" || jiotvServerConfig.TLSKeyPath == "" {
			return fmt.Errorf("TLS cert and key paths are required for HTTPS. Please provide them using --tls-cert and --tls-key flags")
		}
		return app.ListenTLS(fmt.Sprintf("%s:%s", jiotvServerConfig.Host, jiotvServerConfig.Port), jiotvServerConfig.TLSCertPath, jiotvServerConfig.TLSKeyPath)
	} else {
		return app.Listen(fmt.Sprintf("%s:%s", jiotvServerConfig.Host, jiotvServerConfig.Port))
	}
}

// displayHost turns a listen address into one a player can use.
func displayHost(host string) string {
	switch host {
	case "", "0.0.0.0", "[::]", "::":
		return "<this-machine>"
	}
	return host
}
