package cmd

import (
	"fmt"
	"os"

	"github.com/jiotv-go/jiotv_go/v3/internal/access"
	"golang.org/x/term"
)

// SetAdminPassword asks for a new web interface password without echoing it.
func SetAdminPassword() error {
	fd := int(os.Stdin.Fd())
	if !term.IsTerminal(fd) {
		return fmt.Errorf("run this in a terminal")
	}
	fmt.Print("New admin password: ")
	first, err := term.ReadPassword(fd)
	fmt.Println()
	if err != nil {
		return err
	}
	fmt.Print("Repeat it: ")
	second, err := term.ReadPassword(fd)
	fmt.Println()
	if err != nil {
		return err
	}
	if string(first) != string(second) {
		return fmt.Errorf("the passwords don't match")
	}
	if err := access.SetPassword(string(first)); err != nil {
		return err
	}
	fmt.Println("Admin password saved. Existing web sessions are signed out; restart the server if it is running.")
	return nil
}
