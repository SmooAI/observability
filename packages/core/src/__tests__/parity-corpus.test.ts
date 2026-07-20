/**
 * ADR-097 §4 — the TypeScript lane of the parity corpus.
 *
 * Every SDK (TS, Rust, Python, Go, .NET) asserts against the same
 * `parity/sampling-corpus.json` in its own CI. A language that cannot reproduce
 * a vector fails its build. Documentation claiming parity is not evidence.
 */
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { fnv1a32, normalizeLevel, sampleDecision, shouldEmitLog, type CanonicalLevel } from '../sampling';
import { formatTraceparent, parseTraceparent } from '../traceparent';
import { resolveTelemetrySettings } from '../telemetry-settings';

const CORPUS_PATH = join(dirname(fileURLToPath(import.meta.url)), '../../../../parity/sampling-corpus.json');

interface Corpus {
    version: number;
    sampleDecision: { id: string; ratio: number; hash: number; expected: boolean }[];
    sampleDecisionNearThreshold: { id: string; ratio: number; hash: number; position: number; expected: boolean }[];
    sampleDecisionNonFiniteRatio: { id: string; ratio: string; expected: boolean }[];
    levelNormalization: { input: string; expected: string }[];
    traceparentParse: { input: string; expected: { traceId: string; spanId: string; flags: number; sampled: boolean } | null }[];
    traceparentFormat: { input: { traceId: string; spanId: string; flags?: number; sampled?: boolean }; expected: string | null }[];
    settingsResolution: { input: unknown; expected: Record<string, unknown> }[];
    shouldEmitLog: {
        input: { level: string; sessionId: string; traceSampled?: boolean; enabled: boolean; minimumLevel: string; logSamplingRatio: number };
        expected: boolean;
    }[];
}

const corpus: Corpus = JSON.parse(readFileSync(CORPUS_PATH, 'utf8'));

const NON_FINITE: Record<string, number> = { NaN: Number.NaN, Infinity: Number.POSITIVE_INFINITY, '-Infinity': Number.NEGATIVE_INFINITY };

describe('parity corpus', () => {
    it('is the expected version and is not empty', () => {
        expect(corpus.version).toBe(1);
        expect(corpus.sampleDecision.length).toBeGreaterThan(50);
    });

    it.each(corpus.sampleDecision)('sampleDecision($id, $ratio) === $expected', (v) => {
        expect(fnv1a32(v.id)).toBe(v.hash);
        expect(sampleDecision(v.id, v.ratio)).toBe(v.expected);
    });

    it.each(corpus.sampleDecisionNearThreshold)('near-threshold sampleDecision($id, $ratio) === $expected', (v) => {
        expect(fnv1a32(v.id)).toBe(v.hash);
        expect(fnv1a32(v.id) / 2 ** 32).toBeCloseTo(v.position, 12);
        expect(sampleDecision(v.id, v.ratio)).toBe(v.expected);
    });

    it.each(corpus.sampleDecisionNonFiniteRatio)('non-finite ratio $ratio fails open', (v) => {
        expect(sampleDecision(v.id, NON_FINITE[v.ratio]!)).toBe(v.expected);
    });

    it.each(corpus.levelNormalization)('normalizeLevel($input) === $expected', (v) => {
        expect(normalizeLevel(v.input)).toBe(v.expected);
    });

    it.each(corpus.traceparentParse)('parseTraceparent($input)', (v) => {
        expect(parseTraceparent(v.input)).toEqual(v.expected);
    });

    it.each(corpus.traceparentFormat)('formatTraceparent -> $expected', (v) => {
        expect(formatTraceparent(v.input)).toBe(v.expected);
    });

    it.each(corpus.settingsResolution)('resolveTelemetrySettings #%#', (v) => {
        expect(resolveTelemetrySettings(v.input)).toEqual(v.expected);
    });

    it.each(corpus.shouldEmitLog)('shouldEmitLog #%#', (v) => {
        expect(shouldEmitLog({ ...v.input, minimumLevel: v.input.minimumLevel as CanonicalLevel })).toBe(v.expected);
    });
});
