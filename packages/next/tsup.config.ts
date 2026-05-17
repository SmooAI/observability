import { defineConfig } from 'tsup';

export default defineConfig({
    entry: { index: 'src/index.tsx', build: 'src/build.ts' },
    format: ['esm'],
    dts: true,
    clean: true,
    sourcemap: true,
    external: ['react', 'next', 'next/config'],
    target: 'es2022',
});
