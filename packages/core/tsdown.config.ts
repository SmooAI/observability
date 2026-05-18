import { defineConfig } from 'tsdown';

export default defineConfig({
    entry: {
        index: 'src/index.ts',
        browser: 'src/browser/index.ts',
        node: 'src/node/index.ts',
        otel: 'src/otel/index.ts',
        metrics: 'src/metrics/index.ts',
        react: 'src/react/index.tsx',
        next: 'src/next/index.tsx',
        'next-build': 'src/next/build.ts',
    },
    format: ['esm'],
    dts: true,
    clean: true,
    sourcemap: true,
    target: 'es2022',
    // Keep OTel + React + Next as external so consumers' versions are used
    // and the bundle doesn't double-ship them.
    external: [
        '@opentelemetry/api',
        '@opentelemetry/sdk-node',
        '@opentelemetry/auto-instrumentations-node',
        '@opentelemetry/exporter-trace-otlp-http',
        '@opentelemetry/exporter-metrics-otlp-http',
        '@opentelemetry/resources',
        '@opentelemetry/sdk-metrics',
        '@opentelemetry/semantic-conventions',
        'react',
        'react-dom',
        'next',
    ],
});
