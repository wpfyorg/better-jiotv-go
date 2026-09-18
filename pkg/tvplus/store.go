package tvplus

import (
	"encoding/json"
	"errors"

	"github.com/jiotv-go/jiotv_go/v3/pkg/store"
)

// Store keys. They sit next to the JioTV keys in store_v4.toml and never
// overwrite them.
const (
	storeKeyDevice      = "tvplus_device"
	storeKeyCredentials = "tvplus_credentials"
)

// LoadOrCreateDevice returns the saved device, generating and saving one on
// first use so the account sees the same device across restarts.
func LoadOrCreateDevice() (Device, error) {
	var d Device
	if err := loadJSON(storeKeyDevice, &d); err == nil && d.AndroidID != "" {
		return d, nil
	} else if err != nil && !errors.Is(err, store.ErrKeyNotFound) {
		return Device{}, err
	}
	d, err := NewDevice()
	if err != nil {
		return Device{}, err
	}
	return d, saveJSON(storeKeyDevice, d)
}

// LoadCredentials returns the saved login, or nil if there is none.
func LoadCredentials() (*Credentials, error) {
	var cr Credentials
	if err := loadJSON(storeKeyCredentials, &cr); err != nil {
		if errors.Is(err, store.ErrKeyNotFound) {
			return nil, nil
		}
		return nil, err
	}
	if cr.SSOToken == "" {
		return nil, nil
	}
	return &cr, nil
}

// SaveCredentials persists the login.
func SaveCredentials(cr *Credentials) error {
	if cr == nil {
		return errors.New("tvplus: nil credentials")
	}
	return saveJSON(storeKeyCredentials, cr)
}

// DeleteCredentials removes the saved login but keeps the device, so a later
// login reuses the same device slot.
func DeleteCredentials() error {
	return store.Delete(storeKeyCredentials)
}

func loadJSON(key string, v any) error {
	s, err := store.Get(key)
	if err != nil {
		return err
	}
	return json.Unmarshal([]byte(s), v)
}

func saveJSON(key string, v any) error {
	b, err := json.Marshal(v)
	if err != nil {
		return err
	}
	return store.Set(key, string(b))
}
