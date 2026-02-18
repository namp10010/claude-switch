//go:build !darwin

package main

import "encoding/json"

func readKeychainCredentials() json.RawMessage {
	return nil
}

func writeKeychainCredentials(_ *OAuthCredentials) error {
	return nil
}
