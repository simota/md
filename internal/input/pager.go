package input

import (
	"fmt"
	"strings"
)

type PagerMode int

const (
	PagerAuto PagerMode = iota
	PagerAlways
	PagerNever
)

func ParsePagerMode(v string) (PagerMode, error) {
	switch strings.ToLower(strings.TrimSpace(v)) {
	case "", "auto":
		return PagerAuto, nil
	case "always":
		return PagerAlways, nil
	case "never":
		return PagerNever, nil
	default:
		return PagerAuto, fmt.Errorf("invalid --pager=%q (use auto|always|never)", v)
	}
}

func (m PagerMode) ShouldUsePager(stdoutIsTTY bool) bool {
	switch m {
	case PagerAlways:
		return true
	case PagerNever:
		return false
	default:
		return stdoutIsTTY
	}
}

