import { describe, test, expect, beforeAll } from 'bun:test';
import { testClient } from './helpers/client.ts';

let client: Awaited<typeof testClient>;

beforeAll(async () => {
    client = await testClient;
});

describe('Transaction profiling', () => {
    describe('surfnet_getProfileResultsByTag', () => {
        test('returns null for unknown tag', async () => {
            const results = await client.surfnet
                .getProfileResultsByTag('__nonexistent_tag__')
                .send();
            expect(results).toBeNull();
        });
    });
});
