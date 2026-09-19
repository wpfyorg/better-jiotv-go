package web

import (
	"embed"
	"io/fs"
)

//go:embed views/*
var viewFiles embed.FS

//go:embed static/*
var staticFiles embed.FS

//go:embed all:ui/dist
var uiFiles embed.FS

func GetViewFiles() embed.FS {
	return viewFiles
}

func GetStaticFiles() embed.FS {
	return staticFiles
}

// GetUIFiles returns the built Svelte app (web/ui/dist): index.html and assets/.
func GetUIFiles() fs.FS {
	sub, err := fs.Sub(uiFiles, "ui/dist")
	if err != nil {
		panic(err)
	}
	return sub
}
