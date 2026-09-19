package access

import (
	"strings"
	"testing"
	"time"
)

func TestPasswordAndSessions(t *testing.T) {
	setup(t)
	secretMu.Lock()
	secret = nil
	secretMu.Unlock()

	if HasPassword() {
		t.Fatal("fresh store has a password")
	}
	if err := SetPassword("short"); err != ErrWeakPassword {
		t.Errorf("short password: %v", err)
	}
	if err := SetPassword("correct horse"); err != nil {
		t.Fatal(err)
	}
	if ok, err := CheckPassword("correct horse"); !ok || err != nil {
		t.Errorf("right password: %v %v", ok, err)
	}
	if ok, _ := CheckPassword("correct hors"); ok {
		t.Error("wrong password accepted")
	}

	now := time.Now()
	s, err := NewSession(now)
	if err != nil {
		t.Fatal(err)
	}
	if !ValidSession(s, now) {
		t.Error("fresh session rejected")
	}
	if ValidSession(s, now.Add(SessionTTL+time.Second)) {
		t.Error("expired session accepted")
	}
	payload, sig, _ := strings.Cut(s, ".")
	if ValidSession("9"+payload+"."+sig, now) {
		t.Error("tampered session accepted")
	}
	if err := SetPassword("another password"); err != nil {
		t.Fatal(err)
	}
	if ValidSession(s, now) {
		t.Error("session survived a password change")
	}
}

func TestLoginLockout(t *testing.T) {
	setup(t)
	if err := SetPassword("correct horse"); err != nil {
		t.Fatal(err)
	}
	logins.mu.Lock()
	logins.failures = map[string][]time.Time{}
	logins.mu.Unlock()

	now := time.Now()
	for i := 0; i < maxFailures; i++ {
		if ok, err := Login("1.2.3.4", "wrong", now); ok || err != nil {
			t.Fatalf("attempt %d: %v %v", i, ok, err)
		}
	}
	if _, err := Login("1.2.3.4", "correct horse", now); err != ErrTooManyAttempts {
		t.Errorf("locked client: %v", err)
	}
	if ok, err := Login("5.6.7.8", "correct horse", now); !ok || err != nil {
		t.Errorf("other client: %v %v", ok, err)
	}
	if ok, _ := Login("1.2.3.4", "correct horse", now.Add(failureWindow)); !ok {
		t.Error("lockout did not expire")
	}
}
