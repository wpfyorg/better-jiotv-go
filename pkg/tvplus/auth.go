package tvplus

import (
	"crypto/rand"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"time"

	"github.com/valyala/fasthttp"
)

// Values the app sends on the OTP login calls.
const (
	loginAPIKey    = "l7xx61fae40fe3af4c93b02792ae12422a82"
	loginAppKey    = "NzNiMDhlYzQyNjJm"
	loginUserGroup = "tvYR7NSNn7rymo3F"
	loginSessionID = "fa06b053-5b38-4c5b-b9f0-6459827b" // hardcoded in the app
)

// Device is the identity presented to JioTV+. It must stay stable across
// restarts, because each new AndroidID takes a device slot on the account.
type Device struct {
	AndroidID    string `json:"androidId"`
	Model        string `json:"model"`
	Manufacturer string `json:"manufacturer"`
	OSVersion    string `json:"osVersion"`
}

// NewDevice generates a device with a random 16-hex-digit Android ID.
func NewDevice() (Device, error) {
	b := make([]byte, 8)
	if _, err := rand.Read(b); err != nil {
		return Device{}, err
	}
	return Device{
		AndroidID:    hex.EncodeToString(b),
		Model:        "AFTKA",
		Manufacturer: "Amazon",
		OSVersion:    "9",
	}, nil
}

// Credentials are the tokens from a completed login.
type Credentials struct {
	Number       string `json:"number"` // 10-digit mobile number
	SSOToken     string `json:"ssoToken"`
	SubscriberID string `json:"subscriberId"`
	Unique       string `json:"unique"`
	UserID       string `json:"userId"`
	AuthToken    string `json:"authToken"`
	RefreshToken string `json:"refreshToken"`
}

// SendOTPResponse is the sendotp response. FttxIDs lists the fibre
// connections on the number; the second sendotp (after a pick) returns only
// Identifier.
type SendOTPResponse struct {
	Code       int      `json:"code"`
	Message    string   `json:"message"`
	Identifier string   `json:"identifier"`
	FttxIDs    []FttxID `json:"fttxIds"`
}

// FttxID is one fibre customer account.
type FttxID struct {
	CustomerID string `json:"customerId"`
	FirstName  string `json:"firstName"`
	Products   []struct {
		ProductCode string `json:"productCode"`
		ProductName string `json:"productName"`
		Identifier  []struct {
			Name  string `json:"name"`
			Value string `json:"value"`
		} `json:"identifier"`
	} `json:"products"`
}

// Connection is a selectable fibre connection from a sendotp response.
type Connection struct {
	Name        string // account holder's first name
	Identifier  string // MSISDN or FLN value, sent as identifierid
	ProductName string
	CustomerID  string
}

// Connections flattens the fttx list the way the app does: one choice per
// MSISDN or FLN identifier. AirFiber-only lines (R4GID) are not offered.
func (r *SendOTPResponse) Connections() []Connection {
	var out []Connection
	for _, f := range r.FttxIDs {
		for _, p := range f.Products {
			for _, id := range p.Identifier {
				name := strings.ToUpper(id.Name)
				if (name == "MSISDN" || name == "FLN") && id.Value != "" {
					out = append(out, Connection{
						Name:        f.FirstName,
						Identifier:  id.Value,
						ProductName: p.ProductName,
						CustomerID:  f.CustomerID,
					})
				}
			}
		}
	}
	return out
}

// normalizeNumber accepts a 10-digit number with or without +91.
func normalizeNumber(number string) (string, error) {
	n := strings.TrimPrefix(strings.TrimSpace(number), "+91")
	if len(n) != 10 {
		return "", fmt.Errorf("tvplus: mobile number must have 10 digits")
	}
	for _, r := range n {
		if r < '0' || r > '9' {
			return "", fmt.Errorf("tvplus: mobile number must have 10 digits")
		}
	}
	return n, nil
}

func (c *Client) loginHeaders() map[string]string {
	d := c.Device
	return map[string]string{
		"app-name": appName, "x-api-key": loginAPIKey, "x-platform": platform,
		"appkey": loginAppKey, "devicetype": "phone", "os": "android",
		"deviceId": d.AndroidID, "uniqueId": d.AndroidID, "osVersion": d.OSVersion, "dm": d.Model,
		"usergroup": loginUserGroup, "languageId": "6", "userId": "", "sid": loginSessionID,
		"crmid": "", "isott": "false", "channel_id": "-1", "langid": "", "camid": "1",
		"m-rating": "100", "ssotoken": "", "subscriberId": "", "lbcookie": "1",
		"versionCode": versionCode,
	}
}

// SendOTP starts a login. Call it first with an empty identifierID; if the
// response lists Connections, call it again with the chosen Identifier. The
// OTP is sent by one of these calls, and VerifyOTP needs the Identifier from
// the last response.
func (c *Client) SendOTP(number, identifierID string) (*SendOTPResponse, error) {
	n, err := normalizeNumber(number)
	if err != nil {
		return nil, err
	}
	e := c.Endpoints()
	h := c.loginHeaders()
	h["number"] = n
	h["identifierid"] = identifierID
	url := e.Auth + "/" + e.SendOTP
	body, err := c.request(fasthttp.MethodPost, url, h, "application/x-www-form-urlencoded; charset=UTF-8", []byte{})
	if err != nil {
		return nil, err
	}
	var out SendOTPResponse
	if err := decode(url, body, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

type verifyOTPRequest struct {
	DeviceInfo   verifyDeviceInfo `json:"deviceInfo"`
	Identifier   string           `json:"identifier"`
	OTP          string           `json:"otp"`
	RememberUser string           `json:"rememberUser"`
	UpgradeAuth  string           `json:"upgradeAuth"`
}

type verifyDeviceInfo struct {
	ConsumptionDeviceName string `json:"consumptionDeviceName"`
	Info                  struct {
		AndroidID string `json:"androidId"`
		Platform  struct {
			Name string `json:"name"`
		} `json:"platform"`
		Type string `json:"type"`
	} `json:"info"`
}

type verifyOTPResponse struct {
	SSOToken          string `json:"ssoToken"`
	SessionAttributes struct {
		User struct {
			SubscriberID string `json:"subscriberId"`
			Unique       string `json:"unique"`
		} `json:"user"`
	} `json:"sessionAttributes"`
}

// VerifyOTP completes a login and exchanges the SSO token for an access token.
// On success c.Credentials is set. If the exchange fails, the returned
// credentials still hold the SSO token, so the OTP is not wasted.
func (c *Client) VerifyOTP(number, identifier, otp string) (*Credentials, error) {
	n, err := normalizeNumber(number)
	if err != nil {
		return nil, err
	}
	req := verifyOTPRequest{Identifier: identifier, OTP: strings.TrimSpace(otp), RememberUser: "T", UpgradeAuth: "Y"}
	req.DeviceInfo.ConsumptionDeviceName = c.Device.Model
	req.DeviceInfo.Info.AndroidID = c.Device.AndroidID
	req.DeviceInfo.Info.Platform.Name = c.Device.Model
	req.DeviceInfo.Info.Type = "android"

	e := c.Endpoints()
	var v verifyOTPResponse
	if err := c.postJSON(e.Auth+"/"+e.VerifyOTP, c.loginHeaders(), req, &v); err != nil {
		return nil, err
	}
	if v.SSOToken == "" {
		return nil, errors.New("tvplus: verifyotp returned no ssoToken")
	}
	creds := &Credentials{
		Number:       n,
		SSOToken:     v.SSOToken,
		SubscriberID: v.SessionAttributes.User.SubscriberID,
		Unique:       v.SessionAttributes.User.Unique,
	}
	c.Credentials = creds
	if err := c.ExchangeToken(); err != nil {
		return creds, err
	}
	return creds, nil
}

type exchangeResponse struct {
	AuthToken    string `json:"authToken"`
	RefreshToken string `json:"refreshToken"`
	UserID       string `json:"userId"`
	SubscriberID string `json:"subscriberId"`
}

// ExchangeToken turns the SSO token into an access and refresh token.
func (c *Client) ExchangeToken() error {
	cr := c.Credentials
	if cr == nil || cr.SSOToken == "" {
		return errors.New("tvplus: not logged in")
	}
	h := map[string]string{
		"ssotoken": cr.SSOToken, "appname": appName, "deviceid": c.Device.AndroidID,
		"devicetype": "tv", "os": "android", "subscriberid": cr.SubscriberID,
		"persistentRefreshToken": "true", "x-platform": platform,
	}
	payload := map[string]string{"number": base64.StdEncoding.EncodeToString([]byte("+91" + cr.Number))}
	var x exchangeResponse
	if err := c.postJSON(c.Endpoints().UserService+"/loginotp/exchangetoken", h, payload, &x); err != nil {
		return err
	}
	if x.AuthToken == "" {
		return errors.New("tvplus: exchangetoken returned no authToken")
	}
	cr.AuthToken, cr.RefreshToken, cr.UserID = x.AuthToken, x.RefreshToken, x.UserID
	if x.SubscriberID != "" {
		cr.SubscriberID = x.SubscriberID
	}
	return nil
}

// Refresh renews the access token with the refresh token. No OTP is needed.
func (c *Client) Refresh() error {
	cr := c.Credentials
	if cr == nil || cr.RefreshToken == "" {
		return errors.New("tvplus: no refresh token")
	}
	h := map[string]string{
		"accesstoken": cr.AuthToken, "x-platform": platform, "os": "android", "devicetype": "tv",
	}
	payload := map[string]string{"refreshToken": cr.RefreshToken, "appName": appName, "deviceId": c.Device.AndroidID}
	var out struct {
		AuthToken    string `json:"authToken"`
		RefreshToken string `json:"refreshToken"`
	}
	if err := c.postJSON(c.Endpoints().Token+"/refreshtoken", h, payload, &out); err != nil {
		return err
	}
	if out.AuthToken == "" {
		return errors.New("tvplus: refreshtoken returned no authToken")
	}
	cr.AuthToken = out.AuthToken
	if out.RefreshToken != "" {
		cr.RefreshToken = out.RefreshToken
	}
	return nil
}

// AuthTokenExpiry returns the exp claim of the access token, or the zero time
// if it cannot be read.
func (cr *Credentials) AuthTokenExpiry() time.Time {
	if cr == nil {
		return time.Time{}
	}
	parts := strings.Split(cr.AuthToken, ".")
	if len(parts) < 2 {
		return time.Time{}
	}
	payload, err := base64.RawURLEncoding.DecodeString(strings.TrimRight(parts[1], "="))
	if err != nil {
		return time.Time{}
	}
	var claims struct {
		Exp int64 `json:"exp"`
	}
	if json.Unmarshal(payload, &claims) != nil || claims.Exp == 0 {
		return time.Time{}
	}
	return time.Unix(claims.Exp, 0)
}

// NeedsRefresh reports whether the access token expires within margin.
func (cr *Credentials) NeedsRefresh(now time.Time, margin time.Duration) bool {
	exp := cr.AuthTokenExpiry()
	return exp.IsZero() || now.Add(margin).After(exp)
}
