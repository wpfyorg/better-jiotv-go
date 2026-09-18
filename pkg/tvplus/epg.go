package tvplus

import (
	"encoding/json"
	"net/url"
	"strconv"
	"strings"
	"time"

	"github.com/jiotv-go/jiotv_go/v3/pkg/epg"
)

// epgBatchSize is how many channels go in one EPG request. 98 channels with
// three days each come back in about 4.5 MB.
const epgBatchSize = 100

// Programme is one EPG entry from /metadata/v2/livechannels/epg.
type Programme struct {
	Title       string `json:"title"`
	Description string `json:"description"`
	StartEpoch  int64  `json:"startEpoch"` // milliseconds
	EndEpoch    int64  `json:"endEpoch"`   // milliseconds
	Thumbnail   string `json:"thumbnail"`
	ProgramID   string `json:"programId"`
	ShowID      string `json:"showId"`
}

// Start returns the programme start time.
func (p Programme) Start() time.Time { return time.UnixMilli(p.StartEpoch) }

// End returns the programme end time.
func (p Programme) End() time.Time { return time.UnixMilli(p.EndEpoch) }

type epgResponse struct {
	Code    int                    `json:"code"`
	Message string                 `json:"message"`
	Data    map[string][]Programme `json:"data"`
}

// EPG returns programmes keyed by content ID. offsets are days from today
// (0 is today); the app requests 0, 1 and 2. The endpoint needs no login.
func (c *Client) EPG(contentIDs []string, offsets []int) (map[string][]Programme, error) {
	out := make(map[string][]Programme, len(contentIDs))
	if len(contentIDs) == 0 {
		return out, nil
	}
	offs := make([]string, len(offsets))
	for i, o := range offsets {
		offs[i] = strconv.Itoa(o)
	}
	h := commonHeaders()
	h["x-page"] = "Player"
	base := c.Endpoints().Content + "/metadata/v2/livechannels/epg"

	for start := 0; start < len(contentIDs); start += epgBatchSize {
		end := min(start+epgBatchSize, len(contentIDs))
		ids, err := json.Marshal(contentIDs[start:end])
		if err != nil {
			return nil, err
		}
		q := url.Values{}
		// Both parameters are list literals, as the app sends them:
		// contentIds=["300000","302084"]&offsets=[0, 1, 2]
		q.Set("contentIds", string(ids))
		q.Set("offsets", "["+strings.Join(offs, ", ")+"]")
		var r epgResponse
		if err := c.getJSON(base+"?"+q.Encode(), h, &r); err != nil {
			return nil, err
		}
		for id, progs := range r.Data {
			out[id] = append(out[id], progs...)
		}
	}
	return out, nil
}

// ToXMLTV converts programmes for one channel into JioTV Go EPG entries,
// using the prefixed TV+ channel ID. category is the channel's genre.
func ToXMLTV(contentID, category string, progs []Programme) []epg.Programme {
	out := make([]epg.Programme, 0, len(progs))
	for _, p := range progs {
		icon := p.Thumbnail
		if strings.HasSuffix(icon, "/") {
			// Some entries carry only the image folder, not an image.
			icon = ""
		}
		out = append(out, epg.Programme{
			Channel:  ChannelID(contentID),
			Start:    p.Start().Format(xmltvTimeFormat),
			Stop:     p.End().Format(xmltvTimeFormat),
			Title:    epg.Title{Value: p.Title, Lang: "en"},
			Desc:     epg.Desc{Value: p.Description, Lang: "en"},
			Category: epg.Category{Value: category, Lang: "en"},
			Icon:     epg.Icon{Src: icon},
		})
	}
	return out
}

// xmltvTimeFormat matches the format used by pkg/epg.
const xmltvTimeFormat = "20060102150405 -0700"
