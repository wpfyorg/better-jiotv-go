package handlers

import (
	"errors"
	"fmt"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/gofiber/fiber/v2"
	"github.com/jiotv-go/jiotv_go/v3/internal/config"
	internalUtils "github.com/jiotv-go/jiotv_go/v3/internal/utils"
	"github.com/jiotv-go/jiotv_go/v3/pkg/epg"
	"github.com/jiotv-go/jiotv_go/v3/pkg/television"
	"github.com/jiotv-go/jiotv_go/v3/pkg/tvplus"
	"github.com/jiotv-go/jiotv_go/v3/pkg/utils"
	"golang.org/x/sync/singleflight"
)

const (
	// tvPlusChannelsTTL is how long the TV+ catalogue is reused before refetching.
	tvPlusChannelsTTL = 6 * time.Hour
	// tvPlusTokenLead refreshes the 12-hour access token this long before expiry.
	tvPlusTokenLead = time.Hour
)

var (
	errTVPlusDisabled    = errors.New("JioTV+ is not enabled")
	errTVPlusNotLoggedIn = errors.New("JioTV+ is not connected")
)

// tvPlusState holds the JioTV+ client and caches. The client is nil when the
// tvplus option is off.
type tvPlusState struct {
	mu        sync.RWMutex
	client    *tvplus.Client
	catalogue []tvplus.LiveChannel
	fetchedAt time.Time
	extIDs    map[string]string // content ID -> JioTV channel ID, for AES key requests

	// pending holds an unfinished OTP login.
	pending struct {
		number      string
		connections []tvplus.Connection
		identifier  string
	}

	refresh singleflight.Group
}

var tvPlus = &tvPlusState{extIDs: map[string]string{}}

// InitTVPlus sets up the JioTV+ client from the store when the tvplus option
// is on. It is safe to call again after login or logout.
func InitTVPlus() {
	tvPlus.mu.Lock()
	defer tvPlus.mu.Unlock()
	if !config.Cfg.TVPlus {
		tvPlus.client = nil
		return
	}
	if tvPlus.client == nil {
		device, err := tvplus.LoadOrCreateDevice()
		if err != nil {
			utils.Log.Printf("JioTV+: cannot load device: %v", err)
			return
		}
		tvPlus.client = tvplus.NewClient(utils.GetRequestClient(), device)
	}
	creds, err := tvplus.LoadCredentials()
	if err != nil {
		utils.Log.Printf("JioTV+: cannot load login: %v", err)
	}
	tvPlus.client.SetCredentials(creds)
	if creds != nil {
		utils.Log.Println("JioTV+ login loaded")
	}
}

// isTVPlusChannel reports whether a channel ID belongs to JioTV+.
func isTVPlusChannel(channelID string) bool {
	return strings.HasPrefix(channelID, tvplus.IDPrefix)
}

// tvPlusConnected reports whether TV+ is enabled and logged in.
func tvPlusConnected() bool {
	tvPlus.mu.RLock()
	defer tvPlus.mu.RUnlock()
	if tvPlus.client == nil {
		return false
	}
	cr := tvPlus.client.Credentials()
	return cr != nil && cr.AuthToken != ""
}

// getLiveResult returns stream URLs for any channel: JioTV+ IDs go to the TV+
// client, everything else to the JioTV client.
func getLiveResult(channelID string) (*television.LiveURLOutput, error) {
	if isTVPlusChannel(channelID) {
		return tvPlusLive(channelID)
	}
	return TV.Live(channelID)
}

func tvPlusLive(channelID string) (*television.LiveURLOutput, error) {
	client, err := tvPlusClient()
	if err != nil {
		return nil, err
	}
	if err := ensureTVPlusToken(false); err != nil {
		utils.Log.Printf("JioTV+: token refresh failed: %v", err)
	}
	resp, err := client.Playback(channelID)
	if errors.Is(err, tvplus.ErrNotSubscribed) {
		return nil, fmt.Errorf("channel %s is not in your JioTV+ plan", channelID)
	}
	if err != nil {
		return nil, err
	}
	if contentID, ok := tvplus.ContentID(channelID); ok && resp.Data.ExtID != "" {
		tvPlus.mu.Lock()
		tvPlus.extIDs[contentID] = resp.Data.ExtID
		tvPlus.mu.Unlock()
	}
	return resp.LiveURLOutput(), nil
}

// tvPlusClient returns the client if TV+ is enabled and logged in.
func tvPlusClient() (*tvplus.Client, error) {
	tvPlus.mu.RLock()
	defer tvPlus.mu.RUnlock()
	if tvPlus.client == nil {
		return nil, errTVPlusDisabled
	}
	if cr := tvPlus.client.Credentials(); cr == nil || cr.AuthToken == "" {
		return nil, errTVPlusNotLoggedIn
	}
	return tvPlus.client, nil
}

// ensureTVPlusToken refreshes the access token when it is close to expiry, or
// always when force is set. Concurrent callers share one refresh.
func ensureTVPlusToken(force bool) error {
	client, err := tvPlusClient()
	if err != nil {
		return err
	}
	if !force && !client.Credentials().NeedsRefresh(time.Now(), tvPlusTokenLead) {
		return nil
	}
	_, err, _ = tvPlus.refresh.Do("refresh", func() (any, error) {
		if err := client.Refresh(); err != nil {
			return nil, err
		}
		return nil, tvplus.SaveCredentials(client.Credentials())
	})
	return err
}

// RefreshTVPlusTokenTask is the scheduled token upkeep.
func RefreshTVPlusTokenTask() error {
	err := ensureTVPlusToken(false)
	if errors.Is(err, errTVPlusDisabled) || errors.Is(err, errTVPlusNotLoggedIn) {
		return nil
	}
	return err
}

// tvPlusChannels returns the TV+ channels JioTV does not carry, or nil when
// TV+ is off or not logged in. The catalogue is cached for tvPlusChannelsTTL.
func tvPlusChannels(jiotv []television.Channel) []television.Channel {
	client, err := tvPlusClient()
	if err != nil {
		return nil
	}
	tvPlus.mu.RLock()
	catalogue, fresh := tvPlus.catalogue, time.Since(tvPlus.fetchedAt) < tvPlusChannelsTTL
	tvPlus.mu.RUnlock()
	if !fresh {
		fetched, err := client.Channels()
		if err != nil {
			utils.Log.Printf("JioTV+: cannot fetch channels: %v", err)
		} else {
			catalogue = fetched
			tvPlus.mu.Lock()
			tvPlus.catalogue, tvPlus.fetchedAt = fetched, time.Now()
			for _, ch := range fetched {
				if ch.ExtID != "" {
					tvPlus.extIDs[ch.ContentID] = ch.ExtID
				}
			}
			tvPlus.mu.Unlock()
		}
	}
	return tvplus.Exclusive(catalogue, jiotv)
}

// withTVPlusChannels returns the JioTV channels followed by the TV+ ones. The
// input slice is not modified.
func withTVPlusChannels(jiotv []television.Channel) []television.Channel {
	extra := tvPlusChannels(jiotv)
	if len(extra) == 0 {
		return jiotv
	}
	out := make([]television.Channel, 0, len(jiotv)+len(extra))
	out = append(out, jiotv...)
	return append(out, extra...)
}

// tvPlusKeyHeaders returns the headers for an AES-128 key request of a TV+
// channel. ok is false for JioTV channels.
func tvPlusKeyHeaders(channelID string) (map[string]string, bool) {
	contentID, isTVPlus := tvplus.ContentID(channelID)
	if !isTVPlus {
		return nil, false
	}
	client, err := tvPlusClient()
	if err != nil {
		return map[string]string{}, true
	}
	tvPlus.mu.RLock()
	extID := tvPlus.extIDs[contentID]
	tvPlus.mu.RUnlock()
	return client.KeyHeaders(extID), true
}

// tvPlusLicenseHeaders returns the headers for a Widevine license request of a
// TV+ channel. ok is false for JioTV channels.
func tvPlusLicenseHeaders(channelID string) (map[string]string, bool) {
	contentID, isTVPlus := tvplus.ContentID(channelID)
	if !isTVPlus {
		return nil, false
	}
	client, err := tvPlusClient()
	if err != nil {
		return map[string]string{}, true
	}
	return client.LicenseHeaders(tvplus.PlaybackData{ContentID: contentID}), true
}

// TVPlusEPGSource adds TV+ channels and today's and tomorrow's programmes to
// the generated EPG.
func TVPlusEPGSource() ([]epg.Channel, []epg.Programme, error) {
	// EPG generation can start before handlers.Init, so set up TV+ here too.
	tvPlus.mu.RLock()
	initialised := tvPlus.client != nil
	tvPlus.mu.RUnlock()
	if !initialised {
		InitTVPlus()
	}
	client, err := tvPlusClient()
	if err != nil {
		return nil, nil, err
	}
	jiotv, err := television.Channels()
	if err != nil {
		return nil, nil, err
	}
	channels := tvPlusChannels(jiotv.Result)
	ids := make([]string, 0, len(channels))
	for _, ch := range channels {
		id, _ := tvplus.ContentID(ch.ID)
		ids = append(ids, id)
	}
	guide, err := client.EPG(ids, []int{0, 1})
	if err != nil {
		return nil, nil, err
	}
	var epgChannels []epg.Channel
	var programmes []epg.Programme
	for _, ch := range channels {
		id, _ := tvplus.ContentID(ch.ID)
		epgChannels = append(epgChannels, epg.Channel{ID: ch.ID, Display: ch.Name})
		programmes = append(programmes, tvplus.ToXMLTV(id, television.CategoryMap[ch.Category], guide[id])...)
	}
	return epgChannels, programmes, nil
}

// TVPlusSendOTPRequest is the body of POST /tvplus/login/sendOTP. Connection
// is the index of the chosen fibre connection on the second call.
type TVPlusSendOTPRequest struct {
	Number     string `json:"number" xml:"number" form:"number"`
	Connection *int   `json:"connection" xml:"connection" form:"connection"`
}

// TVPlusVerifyOTPRequest is the body of POST /tvplus/login/verifyOTP.
type TVPlusVerifyOTPRequest struct {
	Number string `json:"number" xml:"number" form:"number"`
	OTP    string `json:"otp" xml:"otp" form:"otp"`
}

// tvPlusConnectionView is a fibre connection as shown to the browser. Only
// the last digits of the line number are exposed.
type tvPlusConnectionView struct {
	Index        int    `json:"index"`
	Name         string `json:"name"`
	Product      string `json:"product"`
	LineEndsWith string `json:"lineEndsWith"`
}

// TVPlusSendOTPHandler starts a JioTV+ login. The first call (number only)
// returns the fibre connections on the number when there is a choice; the
// second call (number and connection) sends the OTP for that connection.
func TVPlusSendOTPHandler(c *fiber.Ctx) error {
	tvPlus.mu.RLock()
	client := tvPlus.client
	tvPlus.mu.RUnlock()
	if client == nil {
		return internalUtils.BadRequestError(c, errTVPlusDisabled.Error())
	}
	body := new(TVPlusSendOTPRequest)
	if err := c.BodyParser(body); err != nil {
		return internalUtils.BadRequestError(c, "Invalid JSON")
	}
	number := strings.TrimPrefix(strings.TrimSpace(body.Number), "+91")
	if err := internalUtils.CheckFieldExist(c, "Mobile Number", number != ""); err != nil {
		return err
	}

	if body.Connection != nil {
		tvPlus.mu.RLock()
		pendingNumber, conns := tvPlus.pending.number, tvPlus.pending.connections
		tvPlus.mu.RUnlock()
		i := *body.Connection
		if pendingNumber != number || i < 0 || i >= len(conns) {
			return internalUtils.BadRequestError(c, "Unknown connection, start again")
		}
		resp, err := client.SendOTP(number, conns[i].Identifier)
		if err != nil {
			utils.Log.Printf("JioTV+ sendOTP: %v", err)
			return internalUtils.InternalServerError(c, "Could not send the OTP")
		}
		tvPlus.mu.Lock()
		tvPlus.pending.identifier = resp.Identifier
		tvPlus.mu.Unlock()
		return c.JSON(fiber.Map{"status": resp.Identifier != ""})
	}

	resp, err := client.SendOTP(number, "")
	if err != nil {
		utils.Log.Printf("JioTV+ sendOTP: %v", err)
		return internalUtils.InternalServerError(c, "Could not send the OTP")
	}
	conns := resp.Connections()
	tvPlus.mu.Lock()
	tvPlus.pending.number, tvPlus.pending.connections, tvPlus.pending.identifier = number, conns, resp.Identifier
	tvPlus.mu.Unlock()

	if len(conns) == 0 {
		return c.JSON(fiber.Map{"status": resp.Identifier != ""})
	}
	views := make([]tvPlusConnectionView, len(conns))
	for i, conn := range conns {
		views[i] = tvPlusConnectionView{Index: i, Name: conn.Name, Product: conn.ProductName, LineEndsWith: lastDigits(conn.Identifier, 4)}
	}
	return c.JSON(fiber.Map{"status": true, "connections": views})
}

// TVPlusVerifyOTPHandler completes a JioTV+ login and saves it.
func TVPlusVerifyOTPHandler(c *fiber.Ctx) error {
	tvPlus.mu.RLock()
	client, pending := tvPlus.client, tvPlus.pending
	tvPlus.mu.RUnlock()
	if client == nil {
		return internalUtils.BadRequestError(c, errTVPlusDisabled.Error())
	}
	body := new(TVPlusVerifyOTPRequest)
	if err := c.BodyParser(body); err != nil {
		return internalUtils.BadRequestError(c, "Invalid JSON")
	}
	number := strings.TrimPrefix(strings.TrimSpace(body.Number), "+91")
	if err := internalUtils.CheckFieldExist(c, "OTP", body.OTP != ""); err != nil {
		return err
	}
	if pending.number != number || pending.identifier == "" {
		return internalUtils.BadRequestError(c, "Send the OTP first")
	}

	creds, err := client.VerifyOTP(number, pending.identifier, body.OTP)
	if creds != nil {
		// Save even when the token exchange failed: the SSO token is valid
		// and the exchange can be retried without another OTP.
		if saveErr := tvplus.SaveCredentials(creds); saveErr != nil {
			utils.Log.Printf("JioTV+: cannot save login: %v", saveErr)
		}
	}
	if err != nil {
		utils.Log.Printf("JioTV+ verifyOTP: %v", err)
		return c.JSON(fiber.Map{"status": false})
	}
	tvPlus.mu.Lock()
	tvPlus.pending.number, tvPlus.pending.connections, tvPlus.pending.identifier = "", nil, ""
	tvPlus.fetchedAt = time.Time{}
	tvPlus.mu.Unlock()
	InitTVPlus()
	return c.JSON(fiber.Map{"status": true})
}

// TVPlusLogoutHandler removes the JioTV+ login. The device is kept so a later
// login reuses the same device slot.
func TVPlusLogoutHandler(c *fiber.Ctx) error {
	if !isLogoutDisabled {
		if err := tvplus.DeleteCredentials(); err != nil {
			utils.Log.Printf("JioTV+ logout: %v", err)
		}
		InitTVPlus()
	}
	return c.Redirect("/", fiber.StatusFound)
}

// tvPlusStatus is what the index page shows about JioTV+.
func tvPlusStatus() fiber.Map {
	tvPlus.mu.RLock()
	enabled := tvPlus.client != nil
	tvPlus.mu.RUnlock()
	return fiber.Map{"Enabled": enabled, "Connected": tvPlusConnected()}
}

func lastDigits(s string, n int) string {
	if len(s) <= n {
		return s
	}
	return s[len(s)-n:]
}

// tvPlusWebEPG answers /epg/:channelID/:offset for TV+ channels in the shape
// the web UI reads from the JioTV EPG API.
func tvPlusWebEPG(c *fiber.Ctx, channelID string) error {
	client, err := tvPlusClient()
	if err != nil {
		return fiber.NewError(fiber.StatusNotFound, err.Error())
	}
	offset, err := strconv.Atoi(c.Params("offset"))
	if err != nil || offset < 0 {
		return fiber.NewError(fiber.StatusBadRequest, "Invalid offset")
	}
	contentID, _ := tvplus.ContentID(channelID)
	guide, err := client.EPG([]string{contentID}, []int{offset})
	if err != nil {
		return internalUtils.InternalServerError(c, err.Error())
	}
	type entry struct {
		ShowName      string `json:"showname"`
		Description   string `json:"description"`
		StartEpoch    int64  `json:"startEpoch"`
		EndEpoch      int64  `json:"endEpoch"`
		EpisodePoster string `json:"episodePoster"`
	}
	epgEntries := make([]entry, 0, len(guide[contentID]))
	for _, p := range guide[contentID] {
		poster := p.Thumbnail
		if strings.HasSuffix(poster, "/") {
			poster = ""
		}
		epgEntries = append(epgEntries, entry{ShowName: p.Title, Description: p.Description, StartEpoch: p.StartEpoch, EndEpoch: p.EndEpoch, EpisodePoster: poster})
	}
	return c.JSON(fiber.Map{"epg": epgEntries})
}
