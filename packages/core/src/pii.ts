/**
 * PII scrubbing — applied to message strings, breadcrumb messages, and headers
 * before transport. Stays opinionated and minimal; tenants can extend in
 * `beforeSend`.
 */

const PII_PATTERNS: Array<{ re: RegExp; replacement: string }> = [
    { re: /Bearer\s+[A-Za-z0-9._-]+/gi, replacement: 'Bearer [redacted]' },
    { re: /\b(?:password|passwd|pwd)["']?\s*[:=]\s*["']?[^"'&\s]+/gi, replacement: 'password=[redacted]' },
    { re: /\b(?:token|api[-_]?key|apikey|secret)["']?\s*[:=]\s*["']?[^"'&\s]+/gi, replacement: '$&'.replace(/=.*/, '=[redacted]') },
    { re: /\bsk-[A-Za-z0-9]{20,}/g, replacement: 'sk-[redacted]' },
];

const SENSITIVE_HEADERS = new Set(['authorization', 'cookie', 'set-cookie', 'x-api-key', 'x-auth-token']);

export function scrubString(input: string): string {
    let out = input;
    for (const { re, replacement } of PII_PATTERNS) {
        out = out.replace(re, replacement);
    }
    return out;
}

export function scrubHeaders(headers: Record<string, string> | undefined): Record<string, string> | undefined {
    if (!headers) return headers;
    const out: Record<string, string> = {};
    for (const [k, v] of Object.entries(headers)) {
        out[k] = SENSITIVE_HEADERS.has(k.toLowerCase()) ? '[redacted]' : scrubString(v);
    }
    return out;
}
