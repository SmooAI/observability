import { defineConfig } from 'tsup';

export default defineConfig({
    entry: {
        index: 'src/index.ts',
        browser: 'src/browser/index.ts',
        node: 'src/node/index.ts',
    },
    format: ['esm'],
    dts: true,
    clean: true,
    sourcemap: true,
    splitting: false,
    treeshake: true,
    target: 'es2022',
    external: ['@opentelemetry/api'],
});
