import type { StackFrame } from './types';

/**
 * Parse a JS Error.stack string into structured frames.
 *
 * Supports the three engines we care about:
 *   - V8 (Chrome / Node / Edge)                  "at fn (path:L:C)"
 *   - Spidermonkey (Firefox)                     "fn@path:L:C"
 *   - JavaScriptCore (Safari / older WebKit)     "fn@path:L:C"  (same shape as Firefox)
 *
 * Frames are returned innermost-first to match the @smooai/observability event
 * envelope. SDK-internal frames and frames pointing inside node_modules are
 * tagged `inApp: false`.
 */

const V8_FRAME = /^\s*at\s+(?:(.+?)\s+\()?(.+?)(?::(\d+))?(?::(\d+))?\)?\s*$/;
const GECKO_FRAME = /^(?:(.*?)@)?(.+?)(?::(\d+))?(?::(\d+))?$/;

const SDK_INTERNAL_HINTS = ['@smooai/observability', 'packages/core/dist', 'packages/core/src'];
const NODE_MODULES_RE = /[\\/]node_modules[\\/]/;

export function parseStack(stack: string | undefined): StackFrame[] {
    if (!stack) return [];
    const lines = stack
        .split('\n')
        .map((l) => l.trim())
        .filter(Boolean);
    const frames: StackFrame[] = [];
    for (const line of lines) {
        const frame = parseLine(line);
        if (frame) frames.push(frame);
    }
    return frames;
}

function parseLine(line: string): StackFrame | null {
    // Skip the leading "Error: ..." / "TypeError: ..." line some engines include.
    if (/^(?:[A-Z][A-Za-z]*Error|Error|Uncaught)\b/.test(line)) return null;

    let m = V8_FRAME.exec(line);
    if (m) {
        return makeFrame(m[1], m[2], m[3], m[4]);
    }
    m = GECKO_FRAME.exec(line);
    if (m) {
        return makeFrame(m[1], m[2], m[3], m[4]);
    }
    return null;
}

function makeFrame(fnName: string | undefined, modRaw: string | undefined, linenoRaw: string | undefined, colnoRaw: string | undefined): StackFrame {
    const moduleStr = (modRaw ?? '').replace(/^.*?\((.*)\)$/, '$1').trim() || 'anonymous';
    const lineno = linenoRaw ? Number(linenoRaw) : undefined;
    const colno = colnoRaw ? Number(colnoRaw) : undefined;
    const isInternal = SDK_INTERNAL_HINTS.some((h) => moduleStr.includes(h));
    const isVendor = NODE_MODULES_RE.test(moduleStr);
    return {
        module: moduleStr,
        function: fnName?.trim() || undefined,
        lineno,
        colno,
        inApp: !isInternal && !isVendor,
    };
}

/** Strip SDK-internal frames from the top of a stack. Used by `captureException` */
export function dropSdkFrames(frames: StackFrame[]): StackFrame[] {
    let i = 0;
    while (i < frames.length && frames[i] && frames[i]!.inApp === false && SDK_INTERNAL_HINTS.some((h) => frames[i]!.module.includes(h))) {
        i++;
    }
    return frames.slice(i);
}
