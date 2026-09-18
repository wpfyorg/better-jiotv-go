package tvplus

import (
	"sort"
	"strings"

	"github.com/jiotv-go/jiotv_go/v3/pkg/television"
)

// IDPrefix marks TV+ channel IDs so they never collide with JioTV IDs.
const IDPrefix = "tvp_"

// LiveChannel is one entry of /metadata/v2/livechannels.
type LiveChannel struct {
	ContentID     string   `json:"contentId"`
	ExtID         string   `json:"extId"` // JioTV channel_id when the channel exists there
	Name          string   `json:"name"`
	Language      string   `json:"language"`
	Genres        []string `json:"genres"`
	Quality       string   `json:"quality"` // "HD" or "SD"
	Provider      string   `json:"provider"`
	SubProvider   string   `json:"subProvider"`
	IsPremium     bool     `json:"isPremium"`
	OpenToNonJio  bool     `json:"openToNonJio"`
	PlayViaSDK    bool     `json:"playViaSDK"`
	PlaybackType  string   `json:"playbackType"`
	ChannelNumber int      `json:"channelNumber"`
	Order         int      `json:"order"`
	Thumbnail     string   `json:"thumbnail"`
	LogoURL       string   `json:"logoUrl"`
}

type liveChannelsResponse struct {
	Code    int                    `json:"code"`
	Message string                 `json:"message"`
	Data    map[string]LiveChannel `json:"data"`
}

// Channels returns the full TV+ catalogue, sorted by channel number. The
// endpoint needs no login.
func (c *Client) Channels() ([]LiveChannel, error) {
	h := commonHeaders()
	h["x-page"] = "LiveTv"
	var r liveChannelsResponse
	if err := c.getJSON(c.Endpoints().Content+"/metadata/v2/livechannels", h, &r); err != nil {
		return nil, err
	}
	out := make([]LiveChannel, 0, len(r.Data))
	for id, ch := range r.Data {
		if ch.ContentID == "" {
			ch.ContentID = id
		}
		out = append(out, ch)
	}
	sort.Slice(out, func(i, j int) bool {
		if out[i].ChannelNumber != out[j].ChannelNumber {
			return out[i].ChannelNumber < out[j].ChannelNumber
		}
		return out[i].ContentID < out[j].ContentID
	})
	return out, nil
}

// ChannelID returns the prefixed ID used for a TV+ channel in JioTV Go.
func ChannelID(contentID string) string {
	return IDPrefix + contentID
}

// ContentID strips the TV+ prefix. ok is false for IDs that are not TV+ IDs.
func ContentID(channelID string) (contentID string, ok bool) {
	if !strings.HasPrefix(channelID, IDPrefix) {
		return "", false
	}
	return strings.TrimPrefix(channelID, IDPrefix), true
}

// ToTelevision maps a TV+ channel onto JioTV Go's channel type.
//
// RequiresSubscription stays false: isPremium only says the channel is in a
// paid tier, not whether the logged-in plan includes it, and the playlist
// hides channels that carry the flag.
func (ch LiveChannel) ToTelevision() television.Channel {
	logo := ch.LogoURL
	if logo == "" {
		logo = ch.Thumbnail
	}
	return television.Channel{
		ID:       ChannelID(ch.ContentID),
		Name:     ch.Name,
		LogoURL:  logo,
		Category: categoryID(ch.Genres),
		Language: languageID(ch.Language),
		IsHD:     strings.EqualFold(ch.Quality, "HD"),
	}
}

// isTestChannel filters out Jio's own test entries ("Plus Test2 HD" and so on).
func (ch LiveChannel) isTestChannel() bool {
	return strings.Contains(strings.ToLower(ch.Name), "test")
}

// Exclusive returns the TV+ channels that JioTV does not carry, mapped to
// JioTV Go channels. A TV+ channel is a duplicate when its extId is a JioTV
// channel ID or its normalized name matches a JioTV channel. Duplicates are
// dropped so the JioTV (usually non-DRM) stream is kept.
func Exclusive(tvplus []LiveChannel, jiotv []television.Channel) []television.Channel {
	ids := make(map[string]struct{}, len(jiotv))
	names := make(map[string]struct{}, len(jiotv))
	for _, ch := range jiotv {
		ids[ch.ID] = struct{}{}
		names[normalizeName(ch.Name)] = struct{}{}
	}
	var out []television.Channel
	for _, ch := range tvplus {
		if ch.isTestChannel() || ch.PlaybackType == "deeplink" {
			continue
		}
		if _, dup := ids[ch.ExtID]; dup && ch.ExtID != "" {
			continue
		}
		if _, dup := names[normalizeName(ch.Name)]; dup {
			continue
		}
		out = append(out, ch.ToTelevision())
	}
	return out
}

// normalizeName lowercases a channel name and keeps only letters and digits,
// with "&" read as "and".
func normalizeName(s string) string {
	s = strings.ReplaceAll(strings.ToLower(s), "&", "and")
	var b strings.Builder
	for _, r := range s {
		if (r >= 'a' && r <= 'z') || (r >= '0' && r <= '9') {
			b.WriteRune(r)
		}
	}
	return b.String()
}

// categoryID picks the first genre that JioTV Go knows, or 0 (all).
func categoryID(genres []string) int {
	for _, g := range genres {
		if id, ok := lookup(television.CategoryMap, g); ok {
			return id
		}
	}
	return 0
}

// languageID maps a language name to JioTV Go's ID, or 18 (other).
func languageID(lang string) int {
	if id, ok := lookup(television.LanguageMap, lang); ok {
		return id
	}
	return 18
}

func lookup(m map[int]string, name string) (int, bool) {
	name = strings.TrimSpace(name)
	for id, v := range m {
		if id != 0 && strings.EqualFold(v, name) {
			return id, true
		}
	}
	return 0, false
}
