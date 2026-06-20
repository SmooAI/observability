package observability

import (
	"runtime"
	"strings"
)

// Stack capture for Go. The TS SDK parses Error.stack strings from three JS
// engines; Go has structured stack access via runtime.Callers, so we capture
// frames directly. Frames are returned innermost-first to match the TS event
// envelope (StackFrame[] innermost first).

// maxStackDepth caps captured frames to keep payloads bounded.
const maxStackDepth = 64

// sdkInternalHint marks frames inside this SDK so they can be dropped from the
// top of a capture and tagged inApp=false.
const sdkInternalHint = "github.com/SmooAI/observability/go"

// captureStack walks the call stack starting `skip` frames above the caller of
// captureStack (skip=0 means "the function that called captureStack"). Returns
// frames innermost-first.
func captureStack(skip int) []StackFrame {
	// +2: skip runtime.Callers itself and captureStack.
	pcs := make([]uintptr, maxStackDepth)
	n := runtime.Callers(skip+2, pcs)
	if n == 0 {
		return nil
	}
	frames := runtime.CallersFrames(pcs[:n])
	out := make([]StackFrame, 0, n)
	for {
		fr, more := frames.Next()
		if fr.PC != 0 {
			out = append(out, makeFrame(fr))
		}
		if !more {
			break
		}
	}
	return out
}

// makeFrame converts a runtime.Frame to a StackFrame and decides inApp.
func makeFrame(fr runtime.Frame) StackFrame {
	module := fr.File
	if module == "" {
		module = "unknown"
	}
	fn := fr.Function

	return StackFrame{
		Module:   module,
		Function: fn,
		Lineno:   fr.Line,
		InApp:    isInApp(fn, fr.File),
	}
}

// isInApp returns false for the Go standard library, the runtime, and this SDK.
func isInApp(fn, file string) bool {
	if strings.Contains(fn, sdkInternalHint) {
		return false
	}
	// Standard library frames live under the Go toolchain's src/ and have no
	// domain-style import path (no dot before the first slash in the package).
	if strings.HasPrefix(fn, "runtime.") || strings.HasPrefix(fn, "runtime/") {
		return false
	}
	// Vendored / module-cache dependencies.
	if strings.Contains(file, "/pkg/mod/") {
		return false
	}
	return true
}

// dropSdkFrames strips this SDK's own frames from the top (innermost) of a
// capture so the first reported frame is the caller's code. Mirrors the TS
// dropSdkFrames.
func dropSdkFrames(frames []StackFrame) []StackFrame {
	i := 0
	for i < len(frames) && strings.Contains(frames[i].Function, sdkInternalHint) {
		i++
	}
	return frames[i:]
}
