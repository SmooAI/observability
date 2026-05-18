import { defineConfig } from 'tsup';

export default defineConfig({
    entry: { index: 'src/index.ts' },
    format: ['esm'],
    dts: true,
    clean: true,
    sourcemap: true,
    splitting: false,
    treeshake: true,
    target: 'es2022',
    external: [
        // Auto-instrumentations resolves these dynamically — keep them external
        // so consumers' versions are used.
        '@opentelemetry/api',
        '@opentelemetry/sdk-node',
        '@opentelemetry/auto-instrumentations-node',
        '@opentelemetry/exporter-trace-otlp-http',
        '@opentelemetry/resources',
        '@opentelemetry/semantic-conventions',
    ],
});
