//go:build darwin

package main

import (
	"encoding/json"
	"os"
	"os/exec"
	"strings"
)

func readKeychainCredentials() json.RawMessage {
	account := os.Getenv("USER")
	if account == "" {
		return nil
	}
	out, err := exec.Command("security", "find-generic-password",
		"-s", "Claude Code-credentials", "-a", account, "-w").Output()
	if err != nil {
		return nil
	}
	var doc map[string]json.RawMessage
	if json.Unmarshal([]byte(strings.TrimSpace(string(out))), &doc) != nil {
		return nil
	}
	if raw, ok := doc["claudeAiOauth"]; ok {
		return raw
	}
	return nil
}
