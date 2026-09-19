package access

import (
	"crypto/hmac"
	"crypto/pbkdf2"
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/base64"
	"encoding/hex"
	"errors"
	"fmt"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/gofiber/fiber/v2"
	"github.com/jiotv-go/jiotv_go/v3/pkg/store"
)

const (
	passwordStoreKey = "admin_password"
	secretStoreKey   = "session_secret"

	// SessionCookie holds the signed admin session.
	SessionCookie = "jiotv_session"
	// SessionTTL is how long a login lasts.
	SessionTTL = 30 * 24 * time.Hour

	pbkdf2Iterations = 600_000
	minPasswordLen   = 8

	maxFailures   = 5
	failureWindow = 10 * time.Minute
)

var (
	// ErrWeakPassword is returned for passwords shorter than minPasswordLen.
	ErrWeakPassword = fmt.Errorf("the password needs at least %d characters", minPasswordLen)
	// ErrTooManyAttempts is returned after repeated wrong passwords.
	ErrTooManyAttempts = errors.New("too many wrong passwords, try again later")
	errNoPassword      = errors.New("no admin password set")
)

// HasPassword reports whether an admin password is set.
func HasPassword() bool {
	h, err := store.Get(passwordStoreKey)
	return err == nil && h != ""
}

// SetPassword stores a new admin password and signs out every session.
func SetPassword(password string) error {
	if len(password) < minPasswordLen {
		return ErrWeakPassword
	}
	salt := make([]byte, 16)
	if _, err := rand.Read(salt); err != nil {
		return err
	}
	hash, err := pbkdf2.Key(sha256.New, password, salt, pbkdf2Iterations, 32)
	if err != nil {
		return err
	}
	encoded := fmt.Sprintf("pbkdf2-sha256$%d$%s$%s", pbkdf2Iterations,
		base64.RawStdEncoding.EncodeToString(salt), base64.RawStdEncoding.EncodeToString(hash))
	if err := store.Set(passwordStoreKey, encoded); err != nil {
		return err
	}
	_, err = newSecret()
	return err
}

// CheckPassword reports whether password matches the stored one.
func CheckPassword(password string) (bool, error) {
	encoded, err := store.Get(passwordStoreKey)
	if err != nil || encoded == "" {
		return false, errNoPassword
	}
	parts := strings.Split(encoded, "$")
	if len(parts) != 4 || parts[0] != "pbkdf2-sha256" {
		return false, errors.New("unknown password format")
	}
	iterations, err := strconv.Atoi(parts[1])
	if err != nil {
		return false, err
	}
	salt, err := base64.RawStdEncoding.DecodeString(parts[2])
	if err != nil {
		return false, err
	}
	want, err := base64.RawStdEncoding.DecodeString(parts[3])
	if err != nil {
		return false, err
	}
	got, err := pbkdf2.Key(sha256.New, password, salt, iterations, len(want))
	if err != nil {
		return false, err
	}
	return subtle.ConstantTimeCompare(got, want) == 1, nil
}

var (
	secretMu sync.Mutex
	secret   []byte
)

func sessionSecret() ([]byte, error) {
	secretMu.Lock()
	defer secretMu.Unlock()
	if secret != nil {
		return secret, nil
	}
	if stored, err := store.Get(secretStoreKey); err == nil && stored != "" {
		if b, err := hex.DecodeString(stored); err == nil {
			secret = b
			return secret, nil
		}
	}
	return newSecretLocked()
}

func newSecret() ([]byte, error) {
	secretMu.Lock()
	defer secretMu.Unlock()
	return newSecretLocked()
}

func newSecretLocked() ([]byte, error) {
	b := make([]byte, 32)
	if _, err := rand.Read(b); err != nil {
		return nil, err
	}
	if err := store.Set(secretStoreKey, hex.EncodeToString(b)); err != nil {
		return nil, err
	}
	secret = b
	return secret, nil
}

func sign(payload string) (string, error) {
	s, err := sessionSecret()
	if err != nil {
		return "", err
	}
	mac := hmac.New(sha256.New, s)
	mac.Write([]byte(payload))
	return base64.RawURLEncoding.EncodeToString(mac.Sum(nil)), nil
}

// NewSession returns a signed session value that expires after SessionTTL.
func NewSession(now time.Time) (string, error) {
	payload := strconv.FormatInt(now.Add(SessionTTL).Unix(), 10)
	sig, err := sign(payload)
	if err != nil {
		return "", err
	}
	return payload + "." + sig, nil
}

// ValidSession reports whether value is an unexpired session signed with the
// current secret.
func ValidSession(value string, now time.Time) bool {
	payload, sig, ok := strings.Cut(value, ".")
	if !ok {
		return false
	}
	expires, err := strconv.ParseInt(payload, 10, 64)
	if err != nil || now.Unix() >= expires {
		return false
	}
	want, err := sign(payload)
	return err == nil && hmac.Equal([]byte(sig), []byte(want))
}

// SignOutEverywhere invalidates every session.
func SignOutEverywhere() error {
	_, err := newSecret()
	return err
}

// RequestHasSession reports whether a request carries a valid session cookie.
func RequestHasSession(c *fiber.Ctx) bool {
	return ValidSession(c.Cookies(SessionCookie), time.Now())
}

// limiter counts wrong passwords per client address.
type limiter struct {
	mu       sync.Mutex
	failures map[string][]time.Time
}

var logins = &limiter{failures: map[string][]time.Time{}}

func (l *limiter) recent(ip string, now time.Time) []time.Time {
	var kept []time.Time
	for _, t := range l.failures[ip] {
		if now.Sub(t) < failureWindow {
			kept = append(kept, t)
		}
	}
	l.failures[ip] = kept
	return kept
}

// Login checks a password for a client. After maxFailures wrong passwords in
// failureWindow it refuses without checking.
func Login(ip, password string, now time.Time) (bool, error) {
	logins.mu.Lock()
	if len(logins.recent(ip, now)) >= maxFailures {
		logins.mu.Unlock()
		return false, ErrTooManyAttempts
	}
	logins.mu.Unlock()

	ok, err := CheckPassword(password)
	if err != nil {
		return false, err
	}
	logins.mu.Lock()
	defer logins.mu.Unlock()
	if ok {
		delete(logins.failures, ip)
	} else {
		logins.failures[ip] = append(logins.recent(ip, now), now)
	}
	return ok, nil
}
