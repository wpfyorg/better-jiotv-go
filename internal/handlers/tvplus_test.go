package handlers

import (
	"bytes"
	"crypto/tls"
	"encoding/json"
	"errors"
	"io"
	"log"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/gofiber/fiber/v2"
	"github.com/jiotv-go/jiotv_go/v3/internal/config"
	"github.com/jiotv-go/jiotv_go/v3/pkg/store"
	"github.com/jiotv-go/jiotv_go/v3/pkg/television"
	"github.com/jiotv-go/jiotv_go/v3/pkg/tvplus"
	pkgUtils "github.com/jiotv-go/jiotv_go/v3/pkg/utils"
	"github.com/valyala/fasthttp"
)

func tvPlusFixture(t *testing.T, name string) []byte {
	t.Helper()
	b, err := os.ReadFile(filepath.Join("..", "..", "pkg", "tvplus", "testdata", name))
	if err != nil {
		t.Fatal(err)
	}
	return b
}

// setupTVPlus points the TV+ client at a fake upstream serving the pkg/tvplus
// fixtures, with TV+ enabled and an empty temporary store. It returns the
// headers of the last request per path.
func setupTVPlus(t *testing.T, loggedIn bool) map[string]http.Header {
	t.Helper()
	seen := map[string]http.Header{}
	routes := map[string][]byte{
		"/metadata/v2/livechannels":                   tvPlusFixture(t, "livechannels.json"),
		"/metadata/v2/livechannels/epg":               tvPlusFixture(t, "epg.json"),
		"/apis/v3.2/stbotplogin/verifyotp":            tvPlusFixture(t, "verifyotp.json"),
		"/userservice/apis/v1/loginotp/exchangetoken": tvPlusFixture(t, "exchangetoken.json"),
		"/playback/v2/302084":                         tvPlusFixture(t, "playback.json"),
	}
	upstream := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		seen[r.URL.Path] = r.Header.Clone()
		switch {
		case r.URL.Path == "/apis/v3.2/stbotplogin/sendotp" && r.Header.Get("identifierid") == "":
			w.Write(tvPlusFixture(t, "sendotp.json"))
		case r.URL.Path == "/apis/v3.2/stbotplogin/sendotp":
			w.Write(tvPlusFixture(t, "sendotp_fttx.json"))
		case r.URL.Path == "/playback/v2/300396":
			w.WriteHeader(http.StatusUnauthorized)
			w.Write(tvPlusFixture(t, "playback_401.json"))
		default:
			body, ok := routes[r.URL.Path]
			if !ok {
				http.NotFound(w, r)
				return
			}
			w.Write(body)
		}
	}))
	t.Cleanup(upstream.Close)

	cleanupStore, err := store.SetupTestPathPrefix()
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(cleanupStore)
	if err := store.Init(); err != nil {
		t.Fatal(err)
	}

	previousLog, previousEnabled, previousState := pkgUtils.Log, config.Cfg.TVPlus, tvPlus
	pkgUtils.Log = log.New(io.Discard, "", 0)
	config.Cfg.TVPlus = true
	tvPlus = newTVPlusState()
	t.Cleanup(func() {
		pkgUtils.Log, config.Cfg.TVPlus, tvPlus = previousLog, previousEnabled, previousState
	})

	client := tvplus.NewClient(&fasthttp.Client{TLSConfig: &tls.Config{InsecureSkipVerify: true}},
		tvplus.Device{AndroidID: "0123456789abcdef", Model: "AFTKA", Manufacturer: "Amazon", OSVersion: "9"})
	client.SetEndpoints(tvplus.Endpoints{
		Content: upstream.URL, UserAPI: upstream.URL, Auth: upstream.URL,
		SendOTP: "apis/v3.2/stbotplogin/sendotp", VerifyOTP: "apis/v3.2/stbotplogin/verifyotp",
		UserService: upstream.URL + "/userservice/apis/v1", Token: upstream.URL + "/tokenservice/apis/v1.1",
	})
	if loggedIn {
		client.SetCredentials(&tvplus.Credentials{
			Number: "9000000000", SSOToken: "sso", SubscriberID: "SUB1", Unique: "uniq",
			UserID: "user", AuthToken: "auth", RefreshToken: "refresh",
		})
	}
	tvPlus.client = client
	return seen
}

func postJSONTo(t *testing.T, app *fiber.App, path string, body any) (int, map[string]any) {
	t.Helper()
	b, _ := json.Marshal(body)
	req := httptest.NewRequest(http.MethodPost, path, bytes.NewReader(b))
	req.Header.Set("Content-Type", "application/json")
	resp, err := app.Test(req, -1)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	raw, _ := io.ReadAll(resp.Body)
	out := map[string]any{}
	json.Unmarshal(raw, &out)
	out["_raw"] = string(raw)
	return resp.StatusCode, out
}

func TestGetLiveResultTVPlusStates(t *testing.T) {
	previousEnabled, previousState := config.Cfg.TVPlus, tvPlus
	t.Cleanup(func() { config.Cfg.TVPlus, tvPlus = previousEnabled, previousState })

	tvPlus = newTVPlusState()
	if _, err := getLiveResult("tvp_302084"); !errors.Is(err, errTVPlusDisabled) {
		t.Errorf("disabled: err = %v", err)
	}

	setupTVPlus(t, false)
	if _, err := getLiveResult("tvp_302084"); !errors.Is(err, errTVPlusNotLoggedIn) {
		t.Errorf("logged out: err = %v", err)
	}
}

func TestTVPlusLiveResult(t *testing.T) {
	seen := setupTVPlus(t, true)

	live, err := getLiveResult("tvp_302084")
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(selectBestLiveHLSURL(live, "auto"), "Star_Plus_MOB/Fallback/index.m3u8") {
		t.Errorf("HLS URL = %q", selectBestLiveHLSURL(live, "auto"))
	}
	if !live.HasDRMStream() || live.Hdnea == "" {
		t.Errorf("live result = %+v", live)
	}
	if got := seen["/playback/v2/302084"].Get("x-accesstoken"); got != "auth" {
		t.Errorf("playback x-accesstoken = %q", got)
	}

	if _, err := getLiveResult("tvp_300396"); err == nil || !strings.Contains(err.Error(), "not in your JioTV+ plan") {
		t.Errorf("unsubscribed channel: err = %v", err)
	}
}

func TestWithTVPlusChannels(t *testing.T) {
	jiotv := []television.Channel{{ID: "175", Name: "Aastha"}, {ID: "143", Name: "CNBC TV18 Prime"}}

	previousEnabled, previousState := config.Cfg.TVPlus, tvPlus
	tvPlus = newTVPlusState()
	if got := withTVPlusChannels(jiotv); len(got) != 2 {
		t.Errorf("TV+ disabled: got %d channels", len(got))
	}
	config.Cfg.TVPlus, tvPlus = previousEnabled, previousState

	setupTVPlus(t, true)
	got := withTVPlusChannels(jiotv)
	if len(jiotv) != 2 {
		t.Fatal("input slice changed")
	}
	ids := map[string]bool{}
	for _, ch := range got {
		ids[ch.ID] = true
	}
	if !ids["175"] || !ids["143"] || !ids["tvp_302084"] || ids["tvp_300000"] {
		t.Errorf("channel IDs = %v", ids)
	}

	headers, ok := tvPlusKeyHeaders("tvp_302084")
	if !ok || headers["channelId"] != "1116" || headers["ssotoken"] != "sso" {
		t.Errorf("key headers = %v, %v", headers, ok)
	}
	if _, ok := tvPlusKeyHeaders("143"); ok {
		t.Error("JioTV channel got TV+ key headers")
	}
	if lh, ok := tvPlusLicenseHeaders("tvp_302084"); !ok || lh["channelid"] != "302084" {
		t.Errorf("license headers = %v, %v", lh, ok)
	}
}

func TestTVPlusLoginHandlers(t *testing.T) {
	seen := setupTVPlus(t, false)
	app := fiber.New()
	app.Post("/tvplus/login/sendOTP", TVPlusSendOTPHandler)
	app.Post("/tvplus/login/verifyOTP", TVPlusVerifyOTPHandler)

	if status, _ := postJSONTo(t, app, "/tvplus/login/verifyOTP", map[string]string{"number": "9000000000", "otp": "1"}); status != fiber.StatusBadRequest {
		t.Errorf("verify before send: status %d", status)
	}

	status, out := postJSONTo(t, app, "/tvplus/login/sendOTP", map[string]string{"number": "+919000000000"})
	if status != fiber.StatusOK {
		t.Fatalf("sendOTP status %d: %s", status, out["_raw"])
	}
	conns, _ := out["connections"].([]any)
	if len(conns) != 1 || conns[0].(map[string]any)["lineEndsWith"] != "0001" {
		t.Fatalf("connections = %v", out["connections"])
	}
	if strings.Contains(out["_raw"].(string), "0674000001") {
		t.Error("full line number sent to the browser")
	}

	if status, _ := postJSONTo(t, app, "/tvplus/login/sendOTP", map[string]any{"number": "9000000000", "connection": 5}); status != fiber.StatusBadRequest {
		t.Errorf("bad connection index: status %d", status)
	}
	status, out = postJSONTo(t, app, "/tvplus/login/sendOTP", map[string]any{"number": "9000000000", "connection": 0})
	if status != fiber.StatusOK || out["status"] != true {
		t.Fatalf("second sendOTP = %d %v", status, out["_raw"])
	}
	if got := seen["/apis/v3.2/stbotplogin/sendotp"].Get("identifierid"); got != "0674000001" {
		t.Errorf("identifierid = %q", got)
	}

	status, out = postJSONTo(t, app, "/tvplus/login/verifyOTP", map[string]string{"number": "9000000000", "otp": "123456"})
	if status != fiber.StatusOK || out["status"] != true {
		t.Fatalf("verifyOTP = %d %v", status, out["_raw"])
	}
	saved, err := tvplus.LoadCredentials()
	if err != nil || saved == nil || saved.AuthToken != "FAKE_AUTH_TOKEN" {
		t.Errorf("saved credentials = %+v, %v", saved, err)
	}
	if !tvPlusConnected() {
		t.Error("not connected after login")
	}
}

func TestTVPlusWebEPG(t *testing.T) {
	setupTVPlus(t, true)
	app := fiber.New()
	app.Get("/epg/:channelID/:offset", WebEPGHandler)

	resp, err := app.Test(httptest.NewRequest(http.MethodGet, "/epg/tvp_302084/0", nil), -1)
	if err != nil {
		t.Fatal(err)
	}
	var out struct {
		EPG []struct {
			ShowName   string `json:"showname"`
			StartEpoch int64  `json:"startEpoch"`
			EndEpoch   int64  `json:"endEpoch"`
		} `json:"epg"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&out); err != nil {
		t.Fatal(err)
	}
	if resp.StatusCode != fiber.StatusOK || len(out.EPG) == 0 || out.EPG[0].ShowName == "" || out.EPG[0].EndEpoch <= out.EPG[0].StartEpoch {
		t.Errorf("status %d, epg = %+v", resp.StatusCode, out.EPG)
	}
}

func TestPlaylistServesTVPlusChannelsAsDASH(t *testing.T) {
	previousDRM := EnableDRM
	EnableDRM = true
	t.Cleanup(func() { EnableDRM = previousDRM })

	m3u := GenerateM3UPlaylist([]television.Channel{
		{ID: "tvp_302084", Name: "Star Plus", LogoURL: "https://img.media.jio.com/x.jpg", Category: 5, Language: 1},
	}, "http://host", "", "", "", "", "")
	if !strings.Contains(m3u, "http://host/live/mpd/tvp_302084") ||
		!strings.Contains(m3u, "license_key=http://host/live/key/tvp_302084") {
		t.Errorf("playlist:\n%s", m3u)
	}
	if !strings.Contains(m3u, `tvg-logo="https://img.media.jio.com/x.jpg"`) {
		t.Errorf("logo URL rewritten:\n%s", m3u)
	}

	EnableDRM = false
	m3u = GenerateM3UPlaylist([]television.Channel{{ID: "tvp_302084", Name: "Star Plus"}}, "http://host", "", "", "", "", "")
	if !strings.Contains(m3u, "http://host/live/tvp_302084.m3u8") || strings.Contains(m3u, "KODIPROP") {
		t.Errorf("DRM off, playlist:\n%s", m3u)
	}
}

func TestTVPlusRoute(t *testing.T) {
	setupTVPlus(t, true)
	previousTV := TV
	TV = nil
	t.Cleanup(func() { TV = previousTV })
	tvPlus.catalogue, tvPlus.fetchedAt = []tvplus.LiveChannel{{ContentID: "300396", ExtID: "175"}}, time.Now()
	tvPlus.mirrors = map[string]string{"175": "300396"}

	for id, want := range map[string]string{"tvp_302084": "302084", "175": "300396", "143": ""} {
		got, ok := tvPlusRoute(id)
		if got != want || ok != (want != "") {
			t.Errorf("tvPlusRoute(%q) = %q, %v", id, got, ok)
		}
	}

	TV = &television.Television{AccessToken: "jiotv"}
	if _, ok := tvPlusRoute("175"); ok {
		t.Error("JioTV channel routed to TV+ while JioTV is logged in")
	}
}
