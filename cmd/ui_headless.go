//go:build headless

package cmd

import "github.com/gofiber/fiber/v2"

// Headless reports whether this is the headless build: no web interface or
// player, only the routes IPTV players use.
const Headless = true

func newViews() fiber.Views { return nil }

func registerUI(app *fiber.App) {}
