import { defineConfig } from 'tsup';

export default defineConfig({
    entry: { index: 'src/index.tsx' },
    format: ['esm'],
    dts: true,
    clean: true,
    sourcemap: true,
    external: ['react'],
    target: 'es2022',
});
