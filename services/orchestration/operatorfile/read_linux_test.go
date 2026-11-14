//go:build linux

package operatorfile_test

import (
	"bytes"
	"os"
	"path/filepath"
	"testing"

	"github.com/Emiloart/Celetixo/services/orchestration/operatorfile"
)

func TestPrivateInputsRejectPublicSymlinkHardlinkEmptyAndOversizedFiles(t *testing.T) {
	root := t.TempDir()
	path := filepath.Join(root, "private.json")
	if err := os.WriteFile(path, []byte("private operator input"), 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := operatorfile.Read(path); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(path, 0644); err != nil {
		t.Fatal(err)
	}
	if _, err := operatorfile.Read(path); err == nil {
		t.Fatal("public operator file admitted")
	}
	if err := os.Chmod(path, 0600); err != nil {
		t.Fatal(err)
	}
	alias := filepath.Join(root, "alias")
	if err := os.Symlink(path, alias); err != nil {
		t.Fatal(err)
	}
	if _, err := operatorfile.Read(alias); err == nil {
		t.Fatal("symlink operator file admitted")
	}
	hardlink := filepath.Join(root, "hardlink")
	if err := os.Link(path, hardlink); err != nil {
		t.Fatal(err)
	}
	if _, err := operatorfile.Read(path); err == nil {
		t.Fatal("shared inode operator file admitted")
	}
	if err := os.Remove(hardlink); err != nil {
		t.Fatal(err)
	}
	for _, data := range [][]byte{{}, bytes.Repeat([]byte{'x'}, operatorfile.Limit+1)} {
		if err := os.WriteFile(path, data, 0600); err != nil {
			t.Fatal(err)
		}
		if _, err := operatorfile.Read(path); err == nil {
			t.Fatal("unbounded/empty operator file admitted")
		}
	}
}
