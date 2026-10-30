import { configDefaults, defineConfig } from 'vitest/config';

// Node owns the file-adapter and Cargo integration tests, not Vitest.
export default defineConfig({
    test: {
        exclude: [
            ...configDefaults.exclude,
            'js/open-file.test.mjs',
            'js/example-membership.test.mjs'
        ]
    }
});
