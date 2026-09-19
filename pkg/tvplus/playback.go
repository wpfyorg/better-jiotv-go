package tvplus

import (
	"errors"
	"net/url"
	"strconv"

	"github.com/jiotv-go/jiotv_go/v3/pkg/television"
)

// Streams holds the URLs for each quality level.
type Streams struct {
	Auto   string `json:"auto"`
	High   string `json:"high"`
	Medium string `json:"medium"`
	Low    string `json:"low"`
}

// PlaybackData is the data object of a /playback/v2 response.
type PlaybackData struct {
	ContentID string  `json:"contentId"`
	Name      string  `json:"name"`
	ExtID     string  `json:"extID"`
	M3u8      Streams `json:"m3u8"` // HLS, AES-128 via tv.media.jio.com/fallback keys
	Mpd       struct {
		Streams
		WidevineKeyRotation bool `json:"widevineKeyRotation"`
	} `json:"mpd"` // DASH, Widevine
	KeyURL        string `json:"keyURL"` // Widevine license URL
	AlgoName      string `json:"algoName"`
	PlaybackToken string `json:"playbackToken"`
	Algo          int    `json:"algo"`
	NL            string `json:"nl"`          // ZEE5 license parameter
	PlaybackURL   string `json:"playbackUrl"` // MX Player stream
	TotalDuration int    `json:"totalDuration"`
	ContentType   string `json:"contentType"`
	Provider      string `json:"provider"`
}

// PlaybackResponse is the /playback/v2 response.
type PlaybackResponse struct {
	Code         int          `json:"code"`
	PlaybackCode int          `json:"playbackCode"`
	Message      string       `json:"message"`
	Data         PlaybackData `json:"data"`
}

type playbackRequest struct {
	BitrateProfile string `json:"bitrateProfile"`
	Model          string `json:"model"`
	Manufacturer   string `json:"manufacturer"`
	OSVersion      string `json:"osVersion"`
	SerialNo       string `json:"serialNo"`
	Is4kSupport    bool   `json:"is4kSupport"`
	HevcSupport    bool   `json:"hevcSupport"`
}

// Playback requests stream URLs for a channel. contentID may carry the tvp_
// prefix. Channels outside the account's plan return ErrNotSubscribed.
func (c *Client) Playback(contentID string) (*PlaybackResponse, error) {
	cr := c.Credentials()
	if cr == nil || cr.AuthToken == "" {
		return nil, errors.New("tvplus: not logged in")
	}
	if id, ok := ContentID(contentID); ok {
		contentID = id
	}
	h := commonHeaders()
	h["x-page"] = "Player"
	h["rmn"] = cr.Number
	h["deviceId"] = c.Device.AndroidID
	h["ssotoken"] = cr.SSOToken
	h["uniqueid"] = cr.UserID
	h["subId"] = cr.SubscriberID
	h["x-accesstoken"] = cr.AuthToken

	body := playbackRequest{
		BitrateProfile: "xhdpi",
		Model:          c.Device.Model,
		Manufacturer:   c.Device.Manufacturer,
		OSVersion:      c.Device.OSVersion,
		SerialNo:       c.Device.AndroidID,
		Is4kSupport:    true,
		HevcSupport:    true,
	}
	var r PlaybackResponse
	err := c.postJSON(c.Endpoints().UserAPI+"/playback/v2/"+url.PathEscape(contentID), h, body, &r)
	var apiErr *APIError
	if errors.As(err, &apiErr) && apiErr.Status == 401 {
		return nil, ErrNotSubscribed
	}
	if err != nil {
		return nil, err
	}
	if r.Code == 401 {
		return nil, ErrNotSubscribed
	}
	return &r, nil
}

// LiveURLOutput converts the response into the shape JioTV Go's live and DRM
// handlers already consume.
func (r *PlaybackResponse) LiveURLOutput() *television.LiveURLOutput {
	d := r.Data
	hls := television.Bitrates{Auto: d.M3u8.Auto, High: d.M3u8.High, Medium: d.M3u8.Medium, Low: d.M3u8.Low}
	out := &television.LiveURLOutput{
		Code:     r.Code,
		Message:  r.Message,
		Result:   d.M3u8.Auto,
		Bitrates: hls,
		M3u8:     hls,
		Mpd: television.MPD{
			Auto:   d.Mpd.Auto,
			High:   d.Mpd.High,
			Medium: d.Mpd.Medium,
			Low:    d.Mpd.Low,
			Key:    d.KeyURL,
		},
		IsDRM:    d.KeyURL != "" && d.Mpd.Auto != "",
		KeyURL:   d.KeyURL,
		ExtID:    d.ExtID,
		AlgoName: d.AlgoName,
		Hdnea:    hdneaFrom(d.M3u8.Auto),
	}
	if id, err := strconv.ParseFloat(d.ContentID, 64); err == nil {
		out.ContentID = id
	}
	return out
}

// hdneaFrom returns the __hdnea__ query value of a stream URL.
func hdneaFrom(stream string) string {
	u, err := url.Parse(stream)
	if err != nil {
		return ""
	}
	return u.Query().Get("__hdnea__")
}

// KeyHeaders returns headers for fetching an HLS AES-128 key from
// tv.media.jio.com/fallback. extID is the channel's JioTV ID (PlaybackData.ExtID).
// The request also needs the stream's __hdnea__ as a cookie.
func (c *Client) KeyHeaders(extID string) map[string]string {
	cr := c.Credentials()
	if cr == nil {
		return nil
	}
	return map[string]string{
		"ssotoken":     cr.SSOToken,
		"accesstoken":  cr.AuthToken,
		"srno":         "230203144000",
		"channelId":    extID,
		"subscriberid": cr.SubscriberID,
		"crmid":        cr.SubscriberID,
		"uniqueId":     cr.Unique,
		"deviceId":     c.Device.AndroidID,
		"appkey":       loginAppKey,
		"usergroup":    loginUserGroup,
		"os":           "android",
		"devicetype":   "phone",
		"versionCode":  "422",
	}
}

// LicenseHeaders returns the headers the app sends with Widevine license
// requests to PlaybackData.KeyURL. The license token itself is in the URL.
func (c *Client) LicenseHeaders(d PlaybackData) map[string]string {
	cr := c.Credentials()
	if cr == nil {
		return nil
	}
	return map[string]string{
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
		"subscriberid":  cr.SubscriberID,
		"crmid":         cr.SubscriberID,
	}
}
