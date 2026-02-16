//go:build !darwin

package main

import "encoding/json"

func readKeychainCredentials() json.RawMessage {
	return nil
}
