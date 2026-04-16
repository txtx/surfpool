import { describe, test, expect, beforeAll } from 'bun:test';
import { testClient } from './helpers/client.ts';

let client: Awaited<typeof testClient>;

beforeAll(async () => {
    client = await testClient;
});

describe('Local queries', () => {
    describe('surfnet_getLocalSignatures', () => {
        test('returns an array', async () => {
            const sigs = await client.surfnet.getLocalSignatures().send();
            expect(sigs).toBeArray();
        });

        test('each entry has signature, logs, and err fields', async () => {
            const sigs = await client.surfnet.getLocalSignatures(5).send();
            for (const sig of sigs) {
                expect(typeof sig.signature).toBe('string');
                expect(sig.logs).toBeArray();
                expect(sig.err === null || typeof sig.err === 'object').toBe(true);
            }
        });

        test('limit parameter caps results', async () => {
            const limit = 3;
            const sigs = await client.surfnet.getLocalSignatures(limit).send();
            expect(sigs.length).toBeLessThanOrEqual(limit);
        });
    });
});
