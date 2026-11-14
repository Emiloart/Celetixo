//go:build linux

// Package operatorfile reads bounded private operator input. These paths are
// never accepted from workflow payloads or repository-controlled configuration.
package operatorfile

import (
	"errors"
	"io"
	"os"
	"path/filepath"
	"syscall"
)

const Limit = 64 * 1024

var ErrPrivate = errors.New("operator input must be a bounded private owned regular file")

func Read(path string) ([]byte, error) {
	if !filepath.IsAbs(path) || filepath.Clean(path) != path {
		return nil, ErrPrivate
	}
	canonical, err := filepath.EvalSymlinks(path)
	if err != nil || canonical != path {
		return nil, ErrPrivate
	}
	file, err := os.OpenFile(path, os.O_RDONLY|syscall.O_NOFOLLOW|syscall.O_NONBLOCK, 0)
	if err != nil {
		return nil, ErrPrivate
	}
	defer file.Close()
	info, err := file.Stat()
	if err != nil || !info.Mode().IsRegular() || info.Mode().Perm() != 0600 || info.Size() <= 0 || info.Size() > Limit {
		return nil, ErrPrivate
	}
	stat, ok := info.Sys().(*syscall.Stat_t)
	if !ok || stat.Uid != uint32(os.Geteuid()) || stat.Nlink != 1 {
		return nil, ErrPrivate
	}
	data, err := io.ReadAll(io.LimitReader(file, Limit+1))
	if err != nil || len(data) == 0 || len(data) > Limit {
		return nil, ErrPrivate
	}
	return data, nil
}
