package cmd

import (
	"bufio"
	"fmt"
	"os"
	"strconv"
	"strings"

	"github.com/jiotv-go/jiotv_go/v3/internal/access"
	"github.com/jiotv-go/jiotv_go/v3/pkg/tvplus"
	"github.com/jiotv-go/jiotv_go/v3/pkg/utils"
)

// TVPlusLogin logs in to JioTV+ from the terminal: number, connection, OTP.
// A running server loads the new login on restart.
func TVPlusLogin() error {
	in := bufio.NewReader(os.Stdin)
	ask := func(prompt string) string {
		fmt.Print(prompt)
		line, _ := in.ReadString('\n')
		return strings.TrimSpace(line)
	}

	device, err := tvplus.LoadOrCreateDevice()
	if err != nil {
		return err
	}
	client := tvplus.NewClient(utils.GetRequestClient(), device)

	number := strings.TrimPrefix(ask("Mobile number registered to the fibre account: +91 "), "+91")
	resp, err := client.SendOTP(number, "")
	if err != nil {
		return fmt.Errorf("could not send the OTP: %w", err)
	}
	identifier := resp.Identifier
	if conns := resp.Connections(); len(conns) > 0 {
		fmt.Println("Connections on this number:")
		for i, conn := range conns {
			fmt.Printf("  %d. %s, %s, line ending %s\n", i+1, conn.Name, conn.ProductName, lastDigits(conn.Identifier, 4))
		}
		pick, err := strconv.Atoi(ask("Choose a connection: "))
		if err != nil || pick < 1 || pick > len(conns) {
			return fmt.Errorf("no such connection")
		}
		resp, err = client.SendOTP(number, conns[pick-1].Identifier)
		if err != nil {
			return fmt.Errorf("could not send the OTP: %w", err)
		}
		identifier = resp.Identifier
	}
	if identifier == "" {
		return fmt.Errorf("JioTV+ did not send an OTP")
	}

	otp := ask("OTP: ")
	creds, err := client.VerifyOTP(number, identifier, otp)
	if creds != nil {
		if saveErr := tvplus.SaveCredentials(creds); saveErr != nil {
			return fmt.Errorf("could not save the login: %w", saveErr)
		}
	}
	if err != nil {
		return fmt.Errorf("login failed: %w", err)
	}
	fmt.Println("JioTV+ login saved. Restart the server to use it.")
	return nil
}

// TVPlusLogout deletes the saved JioTV+ login.
func TVPlusLogout() error {
	if err := tvplus.DeleteCredentials(); err != nil {
		return err
	}
	fmt.Println("JioTV+ login deleted.")
	return nil
}

// ShowKey prints the playlist path for the current access key.
func ShowKey() error {
	path, err := access.PlaylistPath()
	if err != nil {
		return err
	}
	fmt.Println("Playlist path:", path)
	fmt.Println("Use it as http://<server>:<port>" + path)
	return nil
}

// RotateKey replaces the access key. Players need the new playlist URL.
func RotateKey() error {
	if _, err := access.Rotate(); err != nil {
		return err
	}
	fmt.Println("New access key created. Restart the server, then update your players.")
	return ShowKey()
}

func lastDigits(s string, n int) string {
	if len(s) <= n {
		return s
	}
	return s[len(s)-n:]
}
