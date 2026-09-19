// Package tvplus talks to the JioTV+ (JioFiber/AirFiber set-top box) APIs.
//
// Endpoints and headers mirror the JioTV+ Android TV app (6.0.8). Base URLs
// come from the remote config (/config/v1) when it can be fetched, with the
// app's build constants as fallback.
package tvplus

import (
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"sync"
	"time"

	"github.com/valyala/fasthttp"
)

// Build constants from the app's AppBuildConfig. They are also the fallback
// when the remote config cannot be fetched.
const (
	DefaultConfigURL      = "https://content-jiotvplus.media.jio.com"
	DefaultUserAPIURL     = "https://api-jiotvplus.media.jio.com"
	DefaultAuthURL        = "https://tv.media.jio.com"
	DefaultUserServiceURL = "https://jiotvapi.media.jio.com/userservice/apis/v1"
	DefaultTokenURL       = "https://auth.media.jio.com/tokenservice/apis/v1.1"

	defaultSendOTPPath   = "apis/v3.2/stbotplogin/sendotp"
	defaultVerifyOTPPath = "apis/v3.2/stbotplogin/verifyotp"

	apiSignature = "37ca682625d7"
	featureCode  = "ce1eb674jdkc"
	appName      = "RJIL_JioTVPlus"
	platform     = "androidtv"
	versionCode  = "6008"
	userAgent    = "ktor-client"

	// PlayerUserAgent is the TV+ app's media player User-Agent. The TV+ CDN
	// refuses DASH manifests and segments unless the User-Agent starts with
	// "JioTV.Plus/".
	PlayerUserAgent = "JioTV.Plus/6.0.8 (Linux;Android 12) AndroidXMedia3/1.4.1"

	// configTTL is how long a fetched remote config is reused.
	configTTL = 24 * time.Hour
)

// ErrNotSubscribed is returned by Playback when the account's plan does not
// include the channel.
var ErrNotSubscribed = errors.New("tvplus: not subscribed")

// Endpoints holds the base URLs used by the client.
type Endpoints struct {
	Content     string // channels, EPG, config
	UserAPI     string // playback, device status
	Auth        string // host for the OTP login paths
	SendOTP     string // path under Auth
	VerifyOTP   string // path under Auth
	UserService string // token exchange
	Token       string // access token refresh
}

// DefaultEndpoints returns the app's build-time endpoints.
func DefaultEndpoints() Endpoints {
	return Endpoints{
		Content:     DefaultConfigURL,
		UserAPI:     DefaultUserAPIURL,
		Auth:        DefaultAuthURL,
		SendOTP:     defaultSendOTPPath,
		VerifyOTP:   defaultVerifyOTPPath,
		UserService: DefaultUserServiceURL,
		Token:       DefaultTokenURL,
	}
}

// Client is a JioTV+ API client. It is safe for concurrent use. Credentials
// are nil until login; use SetCredentials to restore a saved login.
type Client struct {
	HTTP   *fasthttp.Client
	Device Device

	credsMu sync.RWMutex
	creds   *Credentials

	mu        sync.Mutex
	endpoints Endpoints
	fetchedAt time.Time
	now       func() time.Time
}

// NewClient returns a client using the default endpoints.
func NewClient(httpClient *fasthttp.Client, device Device) *Client {
	if httpClient == nil {
		httpClient = &fasthttp.Client{}
	}
	return &Client{
		HTTP:      httpClient,
		Device:    device,
		endpoints: DefaultEndpoints(),
		now:       time.Now,
	}
}

// Credentials returns a copy of the current login, or nil.
func (c *Client) Credentials() *Credentials {
	c.credsMu.RLock()
	defer c.credsMu.RUnlock()
	if c.creds == nil {
		return nil
	}
	cp := *c.creds
	return &cp
}

// SetCredentials replaces the current login. nil logs out.
func (c *Client) SetCredentials(cr *Credentials) {
	var cp *Credentials
	if cr != nil {
		v := *cr
		cp = &v
	}
	c.credsMu.Lock()
	c.creds = cp
	c.credsMu.Unlock()
}

// SetEndpoints overrides the base URLs and marks them as fresh, so the remote
// config is not fetched. Used by tests and for pinning hosts.
func (c *Client) SetEndpoints(e Endpoints) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.endpoints = e
	c.fetchedAt = c.now()
}

// remoteConfig is the part of /config/v1 the client uses.
type remoteConfig struct {
	Data struct {
		APIBasePath             string `json:"apiBasePath"`
		UserSpecificAPIBasePath string `json:"userSpecifcApiBasePath"`
		LoginSendOTP            string `json:"loginSendOtp"`
		LoginVerifyOTP          string `json:"loginVerifyOtp"`
	} `json:"data"`
}

// Endpoints returns the current base URLs, refreshing them from the remote
// config at most once per configTTL. A failed fetch keeps the previous values.
func (c *Client) Endpoints() Endpoints {
	c.mu.Lock()
	defer c.mu.Unlock()
	if !c.fetchedAt.IsZero() && c.now().Sub(c.fetchedAt) < configTTL {
		return c.endpoints
	}
	c.fetchedAt = c.now()

	var cfg remoteConfig
	if err := c.getJSON(c.endpoints.Content+"/config/v1", commonHeaders(), &cfg); err != nil {
		return c.endpoints
	}
	if isHTTPS(cfg.Data.APIBasePath) {
		c.endpoints.Content = strings.TrimRight(cfg.Data.APIBasePath, "/")
	}
	if isHTTPS(cfg.Data.UserSpecificAPIBasePath) {
		c.endpoints.UserAPI = strings.TrimRight(cfg.Data.UserSpecificAPIBasePath, "/")
	}
	if p := strings.Trim(cfg.Data.LoginSendOTP, "/"); p != "" {
		c.endpoints.SendOTP = p
	}
	if p := strings.Trim(cfg.Data.LoginVerifyOTP, "/"); p != "" {
		c.endpoints.VerifyOTP = p
	}
	return c.endpoints
}

func isHTTPS(u string) bool {
	return strings.HasPrefix(u, "https://")
}

// commonHeaders are sent on every content and user API call.
func commonHeaders() map[string]string {
	return map[string]string{
		"x-apisignatures": apiSignature,
		"x-feature-code":  featureCode,
		"x-platform":      platform,
	}
}

// APIError is a non-2xx response from a JioTV+ endpoint.
type APIError struct {
	URL    string
	Status int
	Body   string
}

func (e *APIError) Error() string {
	return fmt.Sprintf("tvplus: %s returned HTTP %d: %s", e.URL, e.Status, e.Body)
}

// request performs an HTTP call and returns the body of a 2xx response.
func (c *Client) request(method, url string, headers map[string]string, contentType string, body []byte) ([]byte, error) {
	req := fasthttp.AcquireRequest()
	defer fasthttp.ReleaseRequest(req)
	resp := fasthttp.AcquireResponse()
	defer fasthttp.ReleaseResponse(resp)

	req.SetRequestURI(url)
	req.Header.SetMethod(method)
	req.Header.SetUserAgent(userAgent)
	for k, v := range headers {
		req.Header.Set(k, v)
	}
	if contentType != "" {
		req.Header.SetContentType(contentType)
	}
	if body != nil {
		req.SetBody(body)
	}

	if err := c.HTTP.DoTimeout(req, resp, 30*time.Second); err != nil {
		return nil, fmt.Errorf("tvplus: %s: %w", url, err)
	}
	out, err := resp.BodyUncompressed()
	if err != nil {
		out = resp.Body()
	}
	out = append([]byte(nil), out...)
	if status := resp.StatusCode(); status < 200 || status >= 300 {
		return out, &APIError{URL: url, Status: status, Body: truncate(string(out), 200)}
	}
	return out, nil
}

func (c *Client) getJSON(url string, headers map[string]string, out any) error {
	body, err := c.request(fasthttp.MethodGet, url, headers, "", nil)
	if err != nil {
		return err
	}
	return decode(url, body, out)
}

func (c *Client) postJSON(url string, headers map[string]string, payload, out any) error {
	b, err := json.Marshal(payload)
	if err != nil {
		return err
	}
	body, err := c.request(fasthttp.MethodPost, url, headers, "application/json; charset=UTF-8", b)
	if err != nil {
		return err
	}
	return decode(url, body, out)
}

func decode(url string, body []byte, out any) error {
	if err := json.Unmarshal(body, out); err != nil {
		return fmt.Errorf("tvplus: decoding %s: %w", url, err)
	}
	return nil
}

func truncate(s string, n int) string {
	if len(s) <= n {
		return s
	}
	return s[:n] + "..."
}
