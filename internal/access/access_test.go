package access

import (
	"io"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/gofiber/fiber/v2"
	"github.com/jiotv-go/jiotv_go/v3/pkg/store"
)

func setup(t *testing.T) (*fiber.App, string) {
	t.Helper()
	cleanup, err := store.SetupTestPathPrefix()
	if err != nil {
		t.Fatal(err)
	}
	if err := store.Init(); err != nil {
		t.Fatal(err)
	}
	mu.Lock()
	key = ""
	mu.Unlock()
	previousCheck := SessionCheck
	t.Cleanup(func() {
		cleanup()
		SessionCheck = previousCheck
		mu.Lock()
		key = ""
		mu.Unlock()
	})

	k, err := Key()
	if err != nil {
		t.Fatal(err)
	}
	app := fiber.New()
	app.Use(Middleware())
	app.Get("/playlist.m3u", func(c *fiber.Ctx) error { return c.SendString(BaseURL(c)) })
	app.Get("/render.m3u8", func(c *fiber.Ctx) error { return c.SendString("open") })
	return app, k
}

func get(t *testing.T, app *fiber.App, path string) (int, string) {
	t.Helper()
	resp, err := app.Test(httptest.NewRequest(http.MethodGet, path, nil))
	if err != nil {
		t.Fatal(err)
	}
	body, _ := io.ReadAll(resp.Body)
	return resp.StatusCode, string(body)
}

func TestKeyIsStoredAndStable(t *testing.T) {
	_, k := setup(t)
	if len(k) != 32 {
		t.Fatalf("key length = %d", len(k))
	}
	mu.Lock()
	key = ""
	mu.Unlock()
	if again, _ := Key(); again != k {
		t.Error("key changed after reload from store")
	}
	rotated, err := Rotate()
	if err != nil || rotated == k {
		t.Errorf("Rotate() = %q, %v", rotated, err)
	}
}

func TestMiddleware(t *testing.T) {
	app, k := setup(t)

	if status, body := get(t, app, "/k/"+k+"/playlist.m3u"); status != 200 || body != "http://example.com/k/"+k {
		t.Errorf("valid key: %d %q", status, body)
	}
	if status, _ := get(t, app, "/k/"+k[:31]+"x/playlist.m3u"); status != 401 {
		t.Errorf("wrong key: status %d", status)
	}
	if status, _ := get(t, app, "/playlist.m3u"); status != 401 {
		t.Errorf("no key: status %d", status)
	}
	if status, body := get(t, app, "/render.m3u8"); status != 200 || body != "open" {
		t.Errorf("open route: %d %q", status, body)
	}

	SessionCheck = func(*fiber.Ctx) bool { return true }
	if status, body := get(t, app, "/playlist.m3u"); status != 200 || body != "http://example.com" {
		t.Errorf("session: %d %q", status, body)
	}
}
