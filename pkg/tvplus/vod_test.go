package tvplus

import "testing"

func TestKeepPlayable(t *testing.T) {
	rails := keepPlayable([]Rail{
		{Title: "Mixed", Items: []VODItem{
			{ContentID: "a", ContentType: "Movie", Provider: "JioCinema", PlaybackType: "playback"},
			{ContentID: "a", ContentType: "Movie", Provider: "JioCinema", PlaybackType: "playback"},
			{ContentID: "b", ContentType: "Show", Provider: "Zee5", PlaybackType: "playback"},
			{ContentID: "c", ContentType: "Movie", Provider: "PrimeVideo", PlaybackType: "deeplink"},
			{ContentID: "d", ContentType: "Movie", Provider: "JioHotstar", PlaybackType: "playback"},
			{ContentID: "e", ContentType: "LiveChannel", Provider: "JioCinema", PlaybackType: "playback"},
			{ContentID: "f", ContentType: "Episode", Provider: "MXPlayer", PlaybackType: "playback"},
		}},
		{Title: "Nothing for us", Items: []VODItem{{ContentID: "g", ContentType: "Movie", Provider: "SonyLIV", PlaybackType: "deeplink"}}},
	})
	if len(rails) != 1 {
		t.Fatalf("rails = %+v", rails)
	}
	var ids []string
	for _, it := range rails[0].Items {
		ids = append(ids, it.ContentID)
	}
	if len(ids) != 3 || ids[0] != "a" || ids[1] != "b" || ids[2] != "f" {
		t.Errorf("kept %v", ids)
	}
}

func TestVODLicenseHeaders(t *testing.T) {
	c := NewClient(nil, testDevice)
	c.SetCredentials(&Credentials{SSOToken: "sso", UserID: "user", SubscriberID: "sub", AuthToken: "auth"})

	jio := c.VODLicenseHeaders(PlaybackData{ContentID: "x", Algo: AlgoJioVOD, PlaybackToken: "pt"})
	if jio["appId"] != "jiovod" || jio["ssoToken"] != "sso" || jio["playbackToken"] != "pt" || jio["channelid"] != "x" {
		t.Errorf("JioCinema headers = %v", jio)
	}
	zee := c.VODLicenseHeaders(PlaybackData{ContentID: "y", Algo: AlgoZee5, PlaybackToken: "pt", NL: "nl"})
	if zee["customData"] != "pt" || zee["nl"] != "nl" || zee["appId"] != "" {
		t.Errorf("ZEE5 headers = %v", zee)
	}
	if NewClient(nil, testDevice).VODLicenseHeaders(PlaybackData{}) != nil {
		t.Error("headers without a login")
	}
}

func TestVODStream(t *testing.T) {
	var d PlaybackData
	d.Mpd.Auto = "https://cdn/a.mpd"
	d.M3u8.Auto = "https://cdn/a.m3u8"
	if u, dash := d.VODStream(); u != "https://cdn/a.mpd" || !dash {
		t.Errorf("DASH first: %q %v", u, dash)
	}
	d.Mpd.Auto = ""
	if u, dash := d.VODStream(); u != "https://cdn/a.m3u8" || dash {
		t.Errorf("HLS: %q %v", u, dash)
	}
	d.M3u8.Auto = ""
	d.PlaybackURL = "https://cdn/b.m3u8"
	if u, dash := d.VODStream(); u != "https://cdn/b.m3u8" || dash {
		t.Errorf("playbackUrl: %q %v", u, dash)
	}
}
