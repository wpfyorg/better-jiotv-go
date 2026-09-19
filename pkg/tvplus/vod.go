package tvplus

import (
	"net/url"
	"strconv"
	"strings"
)

// On-demand titles from the JioTV+ catalogue. Only providers whose streams
// TV+ hands out itself are kept; the rest only open a partner app.

// VODProviders are the providers JioTV Go plays, keyed by the catalogue's
// provider field.
var VODProviders = map[string]string{
	"JioCinema": "JioCinema",
	"MXPlayer":  "MX Player",
	"Zee5":      "ZEE5",
}

// VODItem is a movie, show, episode or clip in the catalogue.
type VODItem struct {
	ContentID      string `json:"contentId"`
	ContentType    string `json:"contentType"` // Movie, Show, Episode, Video, LiveChannel...
	Name           string `json:"name"`
	ShowName       string `json:"showName"`
	Subtitle       string `json:"subtitle"`
	Description    string `json:"description"`
	Thumbnail      string `json:"thumbnail"`
	Portrait       string `json:"portrait"`
	Provider       string `json:"provider"`
	Language       string `json:"language"`
	MaturityRating string `json:"maturityRating"`
	PlaybackType   string `json:"playbackType"`
	Season         int    `json:"season"`
	EpisodeNo      int    `json:"episodeNo"`
	TotalDuration  int    `json:"totalDuration"`
}

// Playable reports whether JioTV Go can play the item or open it (shows).
func (it VODItem) Playable() bool {
	if _, ok := VODProviders[it.Provider]; !ok || it.PlaybackType != "playback" {
		return false
	}
	switch it.ContentType {
	case "Movie", "Show", "Episode", "Video":
		return true
	}
	return false
}

// Rail is a titled row of items.
type Rail struct {
	Title string    `json:"title"`
	Items []VODItem `json:"items"`
}

type railsResponse struct {
	Data     []Rail `json:"data"`
	Metadata struct {
		PageNo     int `json:"pageNo"`
		TotalPages int `json:"totalPages"`
	} `json:"_metadata"`
}

// keepPlayable drops items JioTV Go cannot play, duplicates, and empty rails.
func keepPlayable(rails []Rail) []Rail {
	out := make([]Rail, 0, len(rails))
	for _, r := range rails {
		seen := map[string]bool{}
		var items []VODItem
		for _, it := range r.Items {
			if it.Playable() && !seen[it.ContentID] {
				seen[it.ContentID] = true
				items = append(items, it)
			}
		}
		if len(items) > 0 {
			out = append(out, Rail{Title: r.Title, Items: items})
		}
	}
	return out
}

func contentHeaders(page string) map[string]string {
	h := commonHeaders()
	h["x-page"] = page
	h["x-livetv"] = "no"
	return h
}

// Search returns playable titles matching query, grouped as TV+ groups them
// (Best Match, Movies, TV Show, Videos).
func (c *Client) Search(query string) ([]Rail, error) {
	q := url.Values{}
	q.Set("q", query)
	q.Set("isKids", "false")
	var r railsResponse
	if err := c.getJSON(c.Endpoints().Content+"/search/v1/search?"+q.Encode(), contentHeaders("Search"), &r); err != nil {
		return nil, err
	}
	return keepPlayable(r.Data), nil
}

// Screen returns one page (five rails) of a catalogue screen, such as 1
// (home), 100021 (movies), 100023 (shows) or 100025 (kids). more is false on
// the last page.
func (c *Client) Screen(screenID string, page int) (rails []Rail, more bool, err error) {
	if _, err := strconv.Atoi(screenID); err != nil {
		return nil, false, err
	}
	q := url.Values{}
	q.Set("pageNo", strconv.Itoa(page))
	q.Set("isKids", "false")
	var r railsResponse
	if err := c.getJSON(c.Endpoints().Content+"/screen/v2/"+screenID+"?"+q.Encode(), contentHeaders("Home"), &r); err != nil {
		return nil, false, err
	}
	return keepPlayable(r.Data), len(r.Data) > 0, nil
}

// Episodes returns a show's episodes. season 0 means the default season.
func (c *Client) Episodes(showID string, season int) ([]VODItem, error) {
	u := c.Endpoints().Content + "/metadata/v2/metadata/Show/" + url.PathEscape(showID)
	if season > 0 {
		u += "?season=" + strconv.Itoa(season)
	}
	var r struct {
		Data Rail `json:"data"`
	}
	if err := c.getJSON(u, contentHeaders("Metadata"), &r); err != nil {
		return nil, err
	}
	var out []VODItem
	for _, it := range r.Data.Items {
		if it.ContentType == "Episode" && it.Playable() {
			out = append(out, it)
		}
	}
	return out, nil
}

// Playback algorithms of on-demand content, from playbackResponse.algo.
const (
	AlgoJioVOD = 4  // JioCinema: Widevine DASH, license from Jio
	AlgoZee5   = 6  // ZEE5: Widevine DASH, license from ZEE5
	AlgoMX     = 14 // MX Player: clear HLS
)

// VODLicenseHeaders returns the headers the TV+ app sends with a Widevine
// license request for on-demand content (k2/k.java).
func (c *Client) VODLicenseHeaders(d PlaybackData) map[string]string {
	cr := c.Credentials()
	if cr == nil {
		return nil
	}
	h := map[string]string{
		"os":            "android",
		"playbackToken": d.PlaybackToken,
		"srno":          "230203144000",
		"usergroup":     "474537347347373",
		"deviceid":      c.Device.AndroidID,
		"channelid":     d.ContentID,
		"versionCode":   versionCode,
		"devicetype":    "tv",
		"uniqueid":      cr.UserID,
		"ssotoken":      cr.SSOToken,
	}
	switch d.Algo {
	case AlgoJioVOD:
		h["ssoToken"] = cr.SSOToken
		h["lbCookie"] = ""
		h["idamId"] = ""
		h["jioId"] = ""
		h["deviceId"] = c.Device.AndroidID
		h["appId"] = "jiovod"
		h["appKey"] = "2ccce09e59153fc9"
	case AlgoZee5:
		h["customData"] = d.PlaybackToken
		h["nl"] = d.NL
	}
	return h
}

// VODStream picks the stream to play: DASH when there is one, else HLS.
func (d PlaybackData) VODStream() (streamURL string, dash bool) {
	if d.Mpd.Auto != "" {
		return d.Mpd.Auto, true
	}
	if d.M3u8.Auto != "" {
		return d.M3u8.Auto, false
	}
	return strings.TrimSpace(d.PlaybackURL), strings.Contains(d.PlaybackURL, ".mpd")
}
