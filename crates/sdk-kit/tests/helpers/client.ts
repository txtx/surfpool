import { createSurfpoolClient } from '../../src/index.ts';

const URL = process.env.SURFPOOL_URL ?? 'http://127.0.0.1:8899';

// Shared client instance — awaited once, reused across all test files.
export const testClient = createSurfpoolClient({ url: URL });
