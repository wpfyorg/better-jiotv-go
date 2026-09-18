package tvplus

import (
	"crypto/tls"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/jiotv-go/jiotv_go/v3/internal/config"
	"github.com/jiotv-go/jiotv_go/v3/pkg/store"
	"github.com/jiotv-go/jiotv_go/v3/pkg/television"
	"github.com/valyala/fasthttp"
)

func fixture(t *testing.T, name string) []byte {
	t.Helper()
	b, err := os.ReadFile(filepath.Join("testdata", name))
	if err != nil {
		t.Fatal(err)
	}
	return b
}

var testDevice = Device{AndroidID: "0123456789abcdef", Model: "AFTKA", Manufacturer: "Amazon", OSVersion: "9"}

// testServer serves JioTV+ routes from a handler map and records requests.
type testServer struct {
	*httptest.Server
	mu       sync.Mutex
	requests []*http.Request
	bodies   map[string][]byte
}

func newTestServer(t *testing.T, routes map[string]http.HandlerFunc) (*testServer, *Client) {
	t.Helper()
	ts := &testServer{bodies: map[string][]byte{}}
	ts.Server = httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		ts.mu.Lock()
		ts.requests = append(ts.requests, r)
		ts.bodies[r.URL.Path] = body
		ts.mu.Unlock()
		h, ok := routes[r.URL.Path]
		if !ok {
			http.NotFound(w, r)
			return
		}
		h(w, r)
	}))
	t.Cleanup(ts.Close)

	c := NewClient(&fasthttp.Client{TLSConfig: &tls.Config{InsecureSkipVerify: true}}, testDevice)
	c.SetEndpoints(Endpoints{
		Content:     ts.URL,
		UserAPI:     ts.URL,
		Auth:        ts.URL,
		SendOTP:     "apis/v3.2/stbotplogin/sendotp",
		VerifyOTP:   "apis/v3.2/stbotplogin/verifyotp",
		UserService: ts.URL + "/userservice/apis/v1",
		Token:       ts.URL + "/tokenservice/apis/v1.1",
	})
	return ts, c
}

func (ts *testServer) last(path string) *http.Request {
	ts.mu.Lock()
	defer ts.mu.Unlock()
	for i := len(ts.requests) - 1; i >= 0; i-- {
		if ts.requests[i].URL.Path == path {
			return ts.requests[i]
		}
	}
	return nil
}

func serve(b []byte) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.Write(b)
	}
}

func testCredentials() *Credentials {
	return &Credentials{
		Number: "9000000000", SSOToken: "sso", SubscriberID: "SUB0000001",
		Unique: "uniq", UserID: "user", AuthToken: "auth", RefreshToken: "refresh",
	}
}

// fakeJWT builds an unsigned token with the given exp claim.
func fakeJWT(exp time.Time) string {
	enc := base64.RawURLEncoding.EncodeToString
	return enc([]byte(`{"alg":"none"}`)) + "." + enc([]byte(fmt.Sprintf(`{"exp":%d}`, exp.Unix()))) + ".sig"
}

func TestEndpointsFromRemoteConfig(t *testing.T) {
	fetches := 0
	ts := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		fetches++
		if r.URL.Path != "/config/v1" {
			t.Errorf("unexpected path %s", r.URL.Path)
		}
		if r.Header.Get("x-apisignatures") != apiSignature {
			t.Errorf("missing x-apisignatures")
		}
		w.Write(fixture(t, "remoteconfig.json"))
	}))
	defer ts.Close()

	now := time.Date(2026, 9, 18, 12, 0, 0, 0, time.UTC)
	c := NewClient(&fasthttp.Client{TLSConfig: &tls.Config{InsecureSkipVerify: true}}, testDevice)
	c.now = func() time.Time { return now }
	c.endpoints.Content = ts.URL

	e := c.Endpoints()
	if e.Content != "https://content-jiotvplus.media.jio.com" || e.UserAPI != "https://api-jiotvplus.media.jio.com" {
		t.Errorf("base paths not taken from config: %+v", e)
	}
	if e.SendOTP != "apis/v3.2/stbotplogin/sendotp" || e.VerifyOTP != "apis/v3.2/stbotplogin/verifyotp" {
		t.Errorf("login paths not taken from config: %+v", e)
	}

	c.Endpoints()
	if fetches != 1 {
		t.Errorf("config fetched %d times within TTL, want 1", fetches)
	}
}

func TestEndpointsFallbackOnError(t *testing.T) {
	ts := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Error(w, "down", http.StatusInternalServerError)
	}))
	defer ts.Close()

	c := NewClient(&fasthttp.Client{TLSConfig: &tls.Config{InsecureSkipVerify: true}}, testDevice)
	c.endpoints.Content = ts.URL
	e := c.Endpoints()
	if e.UserAPI != DefaultUserAPIURL || e.SendOTP != defaultSendOTPPath {
		t.Errorf("expected build defaults after failed fetch, got %+v", e)
	}
}

func TestLoginFlow(t *testing.T) {
	sendCalls := 0
	ts, c := newTestServer(t, map[string]http.HandlerFunc{
		"/apis/v3.2/stbotplogin/sendotp": func(w http.ResponseWriter, r *http.Request) {
			sendCalls++
			if r.Header.Get("identifierid") == "" {
				w.Write(fixture(t, "sendotp.json"))
				return
			}
			w.Write(fixture(t, "sendotp_fttx.json"))
		},
		"/apis/v3.2/stbotplogin/verifyotp":            serve(fixture(t, "verifyotp.json")),
		"/userservice/apis/v1/loginotp/exchangetoken": serve(fixture(t, "exchangetoken.json")),
	})

	first, err := c.SendOTP("+919000000000", "")
	if err != nil {
		t.Fatal(err)
	}
	r := ts.last("/apis/v3.2/stbotplogin/sendotp")
	if r.Header.Get("number") != "9000000000" || r.Header.Get("x-api-key") != loginAPIKey || r.Header.Get("deviceId") != testDevice.AndroidID {
		t.Errorf("sendotp headers wrong: number=%q key=%q device=%q", r.Header.Get("number"), r.Header.Get("x-api-key"), r.Header.Get("deviceId"))
	}
	if !strings.HasPrefix(r.Header.Get("Content-Type"), "application/x-www-form-urlencoded") {
		t.Errorf("sendotp content type %q", r.Header.Get("Content-Type"))
	}

	conns := first.Connections()
	if len(conns) != 1 || conns[0].Identifier != "0674000001" || conns[0].ProductName != "JIO HOME VOICE" {
		t.Fatalf("connections = %+v, want only the FLN line", conns)
	}

	second, err := c.SendOTP("9000000000", conns[0].Identifier)
	if err != nil {
		t.Fatal(err)
	}
	if got := ts.last("/apis/v3.2/stbotplogin/sendotp").Header.Get("identifierid"); got != "0674000001" {
		t.Errorf("identifierid = %q", got)
	}
	if sendCalls != 2 {
		t.Errorf("sendotp called %d times", sendCalls)
	}

	creds, err := c.VerifyOTP("9000000000", second.Identifier, " 123456 ")
	if err != nil {
		t.Fatal(err)
	}

	var vb verifyOTPRequest
	if err := json.Unmarshal(ts.bodies["/apis/v3.2/stbotplogin/verifyotp"], &vb); err != nil {
		t.Fatal(err)
	}
	if vb.Identifier != "FAKE_IDENTIFIER_0001" || vb.OTP != "123456" || vb.RememberUser != "T" || vb.UpgradeAuth != "Y" {
		t.Errorf("verifyotp body = %+v", vb)
	}
	if vb.DeviceInfo.Info.AndroidID != testDevice.AndroidID || vb.DeviceInfo.Info.Type != "android" {
		t.Errorf("verifyotp device info = %+v", vb.DeviceInfo)
	}

	x := ts.last("/userservice/apis/v1/loginotp/exchangetoken")
	if x.Header.Get("ssotoken") != "fake.sso.token" || x.Header.Get("subscriberid") != "SUB0000001" {
		t.Errorf("exchange headers wrong")
	}
	var xb map[string]string
	json.Unmarshal(ts.bodies["/userservice/apis/v1/loginotp/exchangetoken"], &xb)
	if want := base64.StdEncoding.EncodeToString([]byte("+919000000000")); xb["number"] != want {
		t.Errorf("exchange number = %q, want %q", xb["number"], want)
	}

	want := Credentials{
		Number: "9000000000", SSOToken: "fake.sso.token", SubscriberID: "SUB0000001",
		Unique: "00000000-0000-0000-0000-000000000001", UserID: "00000000-0000-0000-0000-00000000000u",
		AuthToken: "FAKE_AUTH_TOKEN", RefreshToken: "00000000-0000-0000-0000-00000000000r",
	}
	if *creds != want || c.Credentials != creds {
		t.Errorf("credentials = %+v, want %+v", *creds, want)
	}
}

func TestVerifyOTPKeepsSSOTokenWhenExchangeFails(t *testing.T) {
	_, c := newTestServer(t, map[string]http.HandlerFunc{
		"/apis/v3.2/stbotplogin/verifyotp": serve(fixture(t, "verifyotp.json")),
		"/userservice/apis/v1/loginotp/exchangetoken": func(w http.ResponseWriter, r *http.Request) {
			http.Error(w, "nope", http.StatusBadGateway)
		},
	})
	creds, err := c.VerifyOTP("9000000000", "id", "123456")
	if err == nil {
		t.Fatal("expected exchange error")
	}
	if creds == nil || creds.SSOToken != "fake.sso.token" {
		t.Errorf("SSO token lost after exchange failure: %+v", creds)
	}
}

func TestSendOTPRejectsBadNumber(t *testing.T) {
	c := NewClient(nil, testDevice)
	for _, n := range []string{"", "12345", "90000000001", "90000abcde"} {
		if _, err := c.SendOTP(n, ""); err == nil {
			t.Errorf("SendOTP(%q) accepted", n)
		}
	}
}

func TestRefresh(t *testing.T) {
	ts, c := newTestServer(t, map[string]http.HandlerFunc{
		"/tokenservice/apis/v1.1/refreshtoken": serve([]byte(`{"authToken":"NEW_AUTH"}`)),
	})
	c.Credentials = testCredentials()
	if err := c.Refresh(); err != nil {
		t.Fatal(err)
	}
	r := ts.last("/tokenservice/apis/v1.1/refreshtoken")
	if r.Header.Get("accesstoken") != "auth" || r.Header.Get("devicetype") != "tv" {
		t.Errorf("refresh headers wrong")
	}
	var body map[string]string
	json.Unmarshal(ts.bodies["/tokenservice/apis/v1.1/refreshtoken"], &body)
	if body["refreshToken"] != "refresh" || body["appName"] != appName || body["deviceId"] != testDevice.AndroidID {
		t.Errorf("refresh body = %v", body)
	}
	if c.Credentials.AuthToken != "NEW_AUTH" || c.Credentials.RefreshToken != "refresh" {
		t.Errorf("credentials after refresh = %+v", c.Credentials)
	}
}

func TestAuthTokenExpiry(t *testing.T) {
	now := time.Date(2026, 9, 18, 12, 0, 0, 0, time.UTC)
	cr := &Credentials{AuthToken: fakeJWT(now.Add(12 * time.Hour))}
	if got := cr.AuthTokenExpiry(); !got.Equal(now.Add(12 * time.Hour)) {
		t.Errorf("expiry = %v", got)
	}
	if cr.NeedsRefresh(now, time.Hour) {
		t.Error("fresh token reported as needing refresh")
	}
	if !cr.NeedsRefresh(now.Add(11*time.Hour+30*time.Minute), time.Hour) {
		t.Error("token inside the margin not reported")
	}
	if !(&Credentials{AuthToken: "not-a-jwt"}).NeedsRefresh(now, time.Hour) {
		t.Error("unreadable token should need refresh")
	}
}

func TestChannelsAndExclusive(t *testing.T) {
	ts, c := newTestServer(t, map[string]http.HandlerFunc{
		"/metadata/v2/livechannels": serve(fixture(t, "livechannels.json")),
	})
	chans, err := c.Channels()
	if err != nil {
		t.Fatal(err)
	}
	if len(chans) != 5 {
		t.Fatalf("got %d channels", len(chans))
	}
	for i := 1; i < len(chans); i++ {
		if chans[i-1].ChannelNumber > chans[i].ChannelNumber {
			t.Errorf("channels not sorted by number")
		}
	}
	if ts.last("/metadata/v2/livechannels").Header.Get("x-feature-code") != featureCode {
		t.Error("missing x-feature-code")
	}

	// Aastha exists in JioTV by ID (175); Zee TV by name only.
	jiotv := []television.Channel{{ID: "175", Name: "Aastha TV"}, {ID: "999", Name: "ZEE TV"}}
	got := Exclusive(chans, jiotv)
	ids := map[string]television.Channel{}
	for _, ch := range got {
		ids[ch.ID] = ch
	}
	if _, ok := ids["tvp_300000"]; ok {
		t.Error("Aastha kept despite extId match")
	}
	if _, ok := ids["tvp_300396"]; ok {
		t.Error("Zee TV kept despite name match")
	}
	for _, ch := range chans {
		if ch.isTestChannel() {
			if _, ok := ids[ChannelID(ch.ContentID)]; ok {
				t.Errorf("test channel %q kept", ch.Name)
			}
		}
	}
	star, ok := ids["tvp_302084"]
	if !ok {
		t.Fatalf("Star Plus missing from %v", got)
	}
	if star.Name != "Star Plus" || star.Language != 1 || star.Category != 5 || star.LogoURL == "" || star.RequiresSubscription {
		t.Errorf("Star Plus mapped as %+v", star)
	}
}

func TestContentID(t *testing.T) {
	if id, ok := ContentID("tvp_302084"); !ok || id != "302084" {
		t.Errorf("ContentID(tvp_302084) = %q, %v", id, ok)
	}
	if _, ok := ContentID("143"); ok {
		t.Error("JioTV ID treated as TV+")
	}
}

func TestEPG(t *testing.T) {
	var queries []string
	ts, c := newTestServer(t, map[string]http.HandlerFunc{
		"/metadata/v2/livechannels/epg": func(w http.ResponseWriter, r *http.Request) {
			queries = append(queries, r.URL.Query().Get("contentIds"))
			if got := r.URL.Query().Get("offsets"); got != "[0, 1, 2]" {
				t.Errorf("offsets = %q", got)
			}
			w.Write(fixture(t, "epg.json"))
		},
	})
	_ = ts

	ids := make([]string, 150)
	for i := range ids {
		ids[i] = fmt.Sprint(300000 + i)
	}
	got, err := c.EPG(ids, []int{0, 1, 2})
	if err != nil {
		t.Fatal(err)
	}
	if len(queries) != 2 {
		t.Fatalf("%d requests for 150 channels, want 2", len(queries))
	}
	var first []string
	if err := json.Unmarshal([]byte(queries[0]), &first); err != nil || len(first) != epgBatchSize || first[0] != "300000" {
		t.Errorf("first batch contentIds = %q", queries[0])
	}

	progs := got["302084"]
	if len(progs) != 6 { // 3 per response, two batches
		t.Fatalf("got %d programmes for 302084", len(progs))
	}
	x := ToXMLTV("302084", "Entertainment", progs[:1])
	p := x[0]
	if p.Channel != "tvp_302084" || p.Title.Value != progs[0].Title || p.Category.Value != "Entertainment" {
		t.Errorf("xmltv programme = %+v", p)
	}
	if want := progs[0].Start().Format(xmltvTimeFormat); p.Start != want {
		t.Errorf("start = %q, want %q", p.Start, want)
	}
	if strings.HasSuffix(progs[0].Thumbnail, "/") && p.Icon.Src != "" {
		t.Errorf("folder-only thumbnail used as icon: %q", p.Icon.Src)
	}
}

func TestPlayback(t *testing.T) {
	ts, c := newTestServer(t, map[string]http.HandlerFunc{
		"/playback/v2/302084": serve(fixture(t, "playback.json")),
	})
	c.Credentials = testCredentials()

	resp, err := c.Playback("tvp_302084")
	if err != nil {
		t.Fatal(err)
	}
	r := ts.last("/playback/v2/302084")
	for k, want := range map[string]string{
		"x-page": "Player", "rmn": "9000000000", "deviceId": testDevice.AndroidID, "ssotoken": "sso",
		"uniqueid": "user", "subId": "SUB0000001", "x-accesstoken": "auth", "x-platform": platform,
	} {
		if got := r.Header.Get(k); got != want {
			t.Errorf("header %s = %q, want %q", k, got, want)
		}
	}
	var body playbackRequest
	json.Unmarshal(ts.bodies["/playback/v2/302084"], &body)
	if body.BitrateProfile != "xhdpi" || body.SerialNo != testDevice.AndroidID || !body.HevcSupport {
		t.Errorf("playback body = %+v", body)
	}

	live := resp.LiveURLOutput()
	if !live.IsDRM || !live.HasDRMStream() {
		t.Error("expected a DRM stream")
	}
	if !strings.Contains(live.Mpd.ResolvedBitrates().Auto, "Star_Plus_BTS/WDVLive/index.mpd") {
		t.Errorf("mpd auto = %q", live.Mpd.Auto)
	}
	if !strings.Contains(live.ResolvedLicenseURL(), "/key-delivery/v3/widevine?lt=") {
		t.Errorf("license URL = %q", live.ResolvedLicenseURL())
	}
	if !strings.Contains(live.Result, "Star_Plus_MOB/Fallback/index.m3u8") || live.Bitrates.Auto != live.Result {
		t.Errorf("hls = %q", live.Result)
	}
	if live.ExtID != "1116" || live.ContentID != 302084 || !strings.HasPrefix(live.Hdnea, "st=") {
		t.Errorf("ids/hdnea = %q %v %q", live.ExtID, live.ContentID, live.Hdnea)
	}

	kh := c.KeyHeaders(live.ExtID)
	if kh["channelId"] != "1116" || kh["ssotoken"] != "sso" || kh["uniqueId"] != "uniq" {
		t.Errorf("key headers = %v", kh)
	}
	lh := c.LicenseHeaders(resp.Data)
	if lh["channelid"] != "302084" || lh["uniqueid"] != "user" || lh["usergroup"] != "474537347347373" {
		t.Errorf("license headers = %v", lh)
	}
}

func TestPlaybackNotSubscribed(t *testing.T) {
	_, c := newTestServer(t, map[string]http.HandlerFunc{
		"/playback/v2/300396": func(w http.ResponseWriter, r *http.Request) {
			w.WriteHeader(http.StatusUnauthorized)
			w.Write(fixture(t, "playback_401.json"))
		},
	})
	c.Credentials = testCredentials()
	if _, err := c.Playback("300396"); !errors.Is(err, ErrNotSubscribed) {
		t.Errorf("err = %v, want ErrNotSubscribed", err)
	}
}

func TestPlaybackNeedsLogin(t *testing.T) {
	c := NewClient(nil, testDevice)
	if _, err := c.Playback("302084"); err == nil {
		t.Error("playback without credentials succeeded")
	}
}

func TestStorePersistence(t *testing.T) {
	config.Cfg.PathPrefix = t.TempDir()
	if err := store.Init(); err != nil {
		t.Fatal(err)
	}
	if err := store.Set("ssoToken", "jiotv-sso"); err != nil {
		t.Fatal(err)
	}

	d1, err := LoadOrCreateDevice()
	if err != nil {
		t.Fatal(err)
	}
	d2, err := LoadOrCreateDevice()
	if err != nil {
		t.Fatal(err)
	}
	if d1 != d2 || len(d1.AndroidID) != 16 {
		t.Errorf("device not stable: %+v vs %+v", d1, d2)
	}

	if cr, err := LoadCredentials(); err != nil || cr != nil {
		t.Errorf("LoadCredentials before login = %v, %v", cr, err)
	}
	if err := SaveCredentials(testCredentials()); err != nil {
		t.Fatal(err)
	}
	cr, err := LoadCredentials()
	if err != nil || cr == nil || *cr != *testCredentials() {
		t.Errorf("credentials round trip = %+v, %v", cr, err)
	}

	if err := DeleteCredentials(); err != nil {
		t.Fatal(err)
	}
	if cr, _ := LoadCredentials(); cr != nil {
		t.Error("credentials still present after delete")
	}
	if d3, _ := LoadOrCreateDevice(); d3 != d1 {
		t.Error("device changed after logout")
	}
	if v, _ := store.Get("ssoToken"); v != "jiotv-sso" {
		t.Errorf("JioTV key touched: %q", v)
	}
}
