package handlers

import (
	"fmt"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/gofiber/fiber/v2"
	"github.com/gofiber/fiber/v2/middleware/proxy"
	"github.com/jiotv-go/jiotv_go/v3/internal/access"
	internalUtils "github.com/jiotv-go/jiotv_go/v3/internal/utils"
	"github.com/jiotv-go/jiotv_go/v3/pkg/tvplus"
	"github.com/jiotv-go/jiotv_go/v3/pkg/utils"
)

// On-demand titles from JioTV+ (JioCinema, MX Player, ZEE5). Manifests and
// segments come straight from the providers' CDNs, which allow cross-origin
// requests; only Widevine license requests go through this server, because
// they need the JioTV+ login headers.

// vodPlaybackTTL is how long a playback response (and its license token) is
// reused.
const vodPlaybackTTL = 10 * time.Minute

type vodEntry struct {
	data      tvplus.PlaybackData
	fetchedAt time.Time
}

var vodCache = struct {
	sync.Mutex
	entries map[string]vodEntry
}{entries: map[string]vodEntry{}}

func vodClient(c *fiber.Ctx) (*tvplus.Client, error) {
	client, err := tvPlusClient()
	if err != nil {
		return nil, fiber.NewError(fiber.StatusServiceUnavailable, "connect JioTV+ in Settings to watch on-demand titles")
	}
	return client, nil
}

// vodPlayback returns the playback data for a title, cached for vodPlaybackTTL.
func vodPlayback(client *tvplus.Client, contentID string, fresh bool) (tvplus.PlaybackData, error) {
	vodCache.Lock()
	e, ok := vodCache.entries[contentID]
	vodCache.Unlock()
	if ok && !fresh && time.Since(e.fetchedAt) < vodPlaybackTTL {
		return e.data, nil
	}
	if err := ensureTVPlusToken(false); err != nil {
		utils.Log.Printf("JioTV+: token refresh failed: %v", err)
	}
	resp, err := client.Playback(contentID)
	if err != nil {
		return tvplus.PlaybackData{}, err
	}
	vodCache.Lock()
	for id, old := range vodCache.entries {
		if time.Since(old.fetchedAt) > vodPlaybackTTL {
			delete(vodCache.entries, id)
		}
	}
	vodCache.entries[contentID] = vodEntry{data: resp.Data, fetchedAt: time.Now()}
	vodCache.Unlock()
	return resp.Data, nil
}

func validContentID(id string) bool {
	if id == "" || len(id) > 64 {
		return false
	}
	for _, r := range id {
		if !(r >= '0' && r <= '9' || r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z' || r == '-' || r == '_') {
			return false
		}
	}
	return true
}

// APIOTTSearch searches the on-demand catalogue: GET /api/ott/search?q=
func APIOTTSearch(c *fiber.Ctx) error {
	client, err := vodClient(c)
	if err != nil {
		return err
	}
	q := strings.TrimSpace(c.Query("q"))
	if q == "" {
		return c.JSON(fiber.Map{"rails": []tvplus.Rail{}})
	}
	rails, err := client.Search(q)
	if err != nil {
		utils.Log.Printf("JioTV+ search: %v", err)
		return internalUtils.InternalServerError(c, "search failed")
	}
	return c.JSON(fiber.Map{"rails": rails})
}

// APIOTTScreen returns a page of a catalogue screen: GET /api/ott/screen/:id?page=
func APIOTTScreen(c *fiber.Ctx) error {
	client, err := vodClient(c)
	if err != nil {
		return err
	}
	page, _ := strconv.Atoi(c.Query("page", "0"))
	rails, more, err := client.Screen(c.Params("id"), max(page, 0))
	if err != nil {
		utils.Log.Printf("JioTV+ screen: %v", err)
		return internalUtils.InternalServerError(c, "cannot load this page")
	}
	return c.JSON(fiber.Map{"rails": rails, "more": more})
}

// APIOTTEpisodes lists a show's episodes: GET /api/ott/show/:id?season=
func APIOTTEpisodes(c *fiber.Ctx) error {
	client, err := vodClient(c)
	if err != nil {
		return err
	}
	id := c.Params("id")
	if !validContentID(id) {
		return internalUtils.BadRequestError(c, "invalid id")
	}
	season, _ := strconv.Atoi(c.Query("season", "0"))
	episodes, err := client.Episodes(id, season)
	if err != nil {
		utils.Log.Printf("JioTV+ episodes: %v", err)
		return internalUtils.InternalServerError(c, "cannot load the episodes")
	}
	return c.JSON(fiber.Map{"episodes": episodes})
}

// APIOTTPlay returns what the browser player needs: GET /api/ott/play/:id
func APIOTTPlay(c *fiber.Ctx) error {
	client, err := vodClient(c)
	if err != nil {
		return err
	}
	id := c.Params("id")
	if !validContentID(id) {
		return internalUtils.BadRequestError(c, "invalid id")
	}
	d, err := vodPlayback(client, id, true)
	if err != nil {
		utils.Log.Printf("JioTV+ VOD playback %s: %v", id, err)
		return internalUtils.InternalServerError(c, "this title cannot be played")
	}
	if _, ok := tvplus.VODProviders[d.Provider]; !ok {
		return fiber.NewError(fiber.StatusForbidden, "this provider is not supported")
	}
	stream, dash := d.VODStream()
	if stream == "" {
		return internalUtils.NotFoundError(c, "no stream for this title")
	}
	license := ""
	if d.KeyURL != "" {
		license = "/api/ott/license/" + id
	}
	return c.JSON(fiber.Map{
		"name":     d.Name,
		"provider": tvplus.VODProviders[d.Provider],
		"duration": d.TotalDuration,
		"url":      stream,
		"dash":     dash,
		"license":  license,
	})
}

// APIOTTLicense forwards a Widevine license request to the title's license
// server with the JioTV+ app's headers: POST /api/ott/license/:id and, for
// IPTV players, POST /vod/license/:id.
func APIOTTLicense(c *fiber.Ctx) error {
	client, err := vodClient(c)
	if err != nil {
		return err
	}
	id := c.Params("id")
	if !validContentID(id) {
		return internalUtils.BadRequestError(c, "invalid id")
	}
	d, err := vodPlayback(client, id, false)
	if err != nil || d.KeyURL == "" {
		return internalUtils.NotFoundError(c, "no license for this title")
	}
	if _, ok := tvplus.VODProviders[d.Provider]; !ok {
		return fiber.NewError(fiber.StatusForbidden, "this provider is not supported")
	}
	for key, value := range client.VODLicenseHeaders(d) {
		c.Request().Header.Set(key, value)
	}
	c.Request().Header.Set("User-Agent", tvplus.PlayerUserAgent)
	c.Request().Header.Set("Content-Type", "application/octet-stream")
	for _, h := range []string{"Accept", "Origin", "Referer", "Cookie"} {
		c.Request().Header.Del(h)
	}
	if err := proxy.Do(c, d.KeyURL, TV.Client); err != nil {
		return err
	}
	c.Response().Header.Del(fiber.HeaderServer)
	c.Response().Header.Del(fiber.HeaderSetCookie)
	return nil
}

// VODStreamHandler sends an IPTV player to a fresh stream URL for a title:
// GET /vod/:id
func VODStreamHandler(c *fiber.Ctx) error {
	client, err := vodClient(c)
	if err != nil {
		return err
	}
	id := strings.TrimSuffix(strings.TrimSuffix(c.Params("id"), ".mpd"), ".m3u8")
	if !validContentID(id) {
		return internalUtils.BadRequestError(c, "invalid id")
	}
	d, err := vodPlayback(client, id, true)
	if err != nil {
		utils.Log.Printf("JioTV+ VOD playback %s: %v", id, err)
		return internalUtils.InternalServerError(c, "this title cannot be played")
	}
	if _, ok := tvplus.VODProviders[d.Provider]; !ok {
		return fiber.NewError(fiber.StatusForbidden, "this provider is not supported")
	}
	stream, _ := d.VODStream()
	if stream == "" {
		return internalUtils.NotFoundError(c, "no stream for this title")
	}
	return c.Redirect(stream, fiber.StatusFound)
}

// vodPlaylistTTL is how long the generated on-demand playlist is reused.
const vodPlaylistTTL = 6 * time.Hour

var vodPlaylist = struct {
	sync.Mutex
	entries   []vodPlaylistEntry
	fetchedAt time.Time
}{}

type vodPlaylistEntry struct {
	item  tvplus.VODItem
	group string
}

// vodPlaylistScreens are the catalogue screens the playlist is built from,
// with how many pages of each to read.
var vodPlaylistScreens = []struct {
	id    string
	pages int
}{{"1", 4}, {"100021", 6}, {"100023", 6}, {"100025", 4}, {"100097", 4}}

// maxPlaylistShows caps how many shows are expanded into episodes.
const maxPlaylistShows = 40

func buildVODPlaylist(client *tvplus.Client) []vodPlaylistEntry {
	var out []vodPlaylistEntry
	seen := map[string]bool{}
	shows := 0
	add := func(it tvplus.VODItem, group string) {
		if !seen[it.ContentID] {
			seen[it.ContentID] = true
			out = append(out, vodPlaylistEntry{item: it, group: group})
		}
	}
	for _, sc := range vodPlaylistScreens {
		for page := 0; page < sc.pages; page++ {
			rails, more, err := client.Screen(sc.id, page)
			if err != nil {
				utils.Log.Printf("JioTV+ VOD playlist, screen %s: %v", sc.id, err)
				break
			}
			for _, r := range rails {
				for _, it := range r.Items {
					provider := tvplus.VODProviders[it.Provider]
					if it.ContentType != "Show" {
						add(it, provider+" · "+r.Title)
						continue
					}
					if seen[it.ContentID] || shows >= maxPlaylistShows {
						continue
					}
					seen[it.ContentID] = true
					shows++
					episodes, err := client.Episodes(it.ContentID, 0)
					if err != nil {
						continue
					}
					for _, ep := range episodes {
						if ep.ShowName == "" {
							ep.ShowName = it.Name
						}
						add(ep, provider+" · "+it.Name)
					}
				}
			}
			if !more {
				break
			}
		}
	}
	return out
}

// VODPlaylistHandler serves on-demand titles as an M3U playlist: GET /vod.m3u
func VODPlaylistHandler(c *fiber.Ctx) error {
	client, err := vodClient(c)
	if err != nil {
		return err
	}
	vodPlaylist.Lock()
	if vodPlaylist.entries == nil || time.Since(vodPlaylist.fetchedAt) > vodPlaylistTTL {
		if entries := buildVODPlaylist(client); len(entries) > 0 {
			vodPlaylist.entries, vodPlaylist.fetchedAt = entries, time.Now()
		}
	}
	entries := vodPlaylist.entries
	vodPlaylist.Unlock()

	base := access.BaseURL(c)
	var b strings.Builder
	b.WriteString("#EXTM3U\n")
	for _, e := range entries {
		it := e.item
		name := it.Name
		if it.ContentType == "Episode" && it.ShowName != "" {
			name = fmt.Sprintf("%s S%02dE%02d %s", it.ShowName, max(it.Season, 1), it.EpisodeNo, it.Name)
		}
		name = strings.NewReplacer("\n", " ", ",", " ").Replace(name)
		group := strings.ReplaceAll(e.group, "\"", "'")
		fmt.Fprintf(&b, "#EXTINF:%d tvg-id=\"vod_%s\" tvg-logo=\"%s\" group-title=\"%s\",%s\n",
			max(it.TotalDuration, -1), it.ContentID, it.Thumbnail, group, name)
		if it.Provider == "MXPlayer" {
			fmt.Fprintf(&b, "%s/vod/%s.m3u8\n", base, it.ContentID)
			continue
		}
		b.WriteString("#KODIPROP:inputstream=inputstream.adaptive\n")
		b.WriteString("#KODIPROP:inputstream.adaptive.manifest_type=mpd\n")
		b.WriteString("#KODIPROP:inputstream.adaptive.license_type=com.widevine.alpha\n")
		fmt.Fprintf(&b, "#KODIPROP:inputstream.adaptive.license_key=%s/vod/license/%s\n", base, it.ContentID)
		fmt.Fprintf(&b, "%s/vod/%s.mpd\n", base, it.ContentID)
	}
	c.Set(fiber.HeaderContentType, "audio/x-mpegurl; charset=utf-8")
	c.Set(fiber.HeaderContentDisposition, "inline; filename=\"vod.m3u\"")
	return c.SendString(b.String())
}
