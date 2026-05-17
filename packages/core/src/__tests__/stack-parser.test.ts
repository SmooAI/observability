import { describe, expect, it } from 'vitest';
import { parseStack } from '../stack-parser';

describe('parseStack', () => {
    it('parses a V8 (Chrome) stack', () => {
        const stack = [
            "TypeError: Cannot read properties of undefined (reading 'call')",
            '    at s (webpack-766ccbbf0ad1fc08.js:1:161)',
            '    at c (90656-ecba1b78b94a6f78.js:32:21693)',
            '    at L (90656-ecba1b78b94a6f78.js:32:25222)',
        ].join('\n');
        const frames = parseStack(stack);
        expect(frames).toHaveLength(3);
        expect(frames[0]).toMatchObject({ function: 's', module: 'webpack-766ccbbf0ad1fc08.js', lineno: 1, colno: 161, inApp: true });
    });

    it('parses a Spidermonkey (Firefox) stack', () => {
        const stack = ['fn@http://localhost/app.js:42:7', 'doWork@http://localhost/app.js:100:1'].join('\n');
        const frames = parseStack(stack);
        expect(frames).toHaveLength(2);
        expect(frames[0]).toMatchObject({ function: 'fn' });
    });

    it('returns empty for missing stack', () => {
        expect(parseStack(undefined)).toEqual([]);
        expect(parseStack('')).toEqual([]);
    });

    it('flags node_modules frames as non-app', () => {
        const stack = '    at Object.foo (/app/node_modules/react/index.js:1:2)';
        const frames = parseStack(stack);
        expect(frames[0]).toMatchObject({ inApp: false });
    });

    it('skips leading "Error: ..." header', () => {
        const stack = ['Error: boom', '    at fn (file.js:1:1)'].join('\n');
        const frames = parseStack(stack);
        expect(frames).toHaveLength(1);
    });
});
