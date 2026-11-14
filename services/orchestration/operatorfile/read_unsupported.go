//go:build !linux

package operatorfile

import "errors"

const Limit = 64 * 1024

var ErrPrivate = errors.New("private operator input requires the validated Linux profile")

func Read(string) ([]byte, error) { return nil, ErrPrivate }
