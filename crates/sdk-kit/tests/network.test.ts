import { describe, test, expect, beforeAll } from 'bun:test';
import { testClient } from './helpers/client.ts';
import { assertNullResponse } from './helpers/assertions.ts';

let client: Awaited<typeof testClient>;

beforeAll(async () => {
    client = await testClient;
});

describe('Network control', () => {
    describe('surfnet_getSurfnetInfo', () => {
        test('returns info object', async () => {
            const info = await client.surfnet.getSurfnetInfo().send();
            expect(info).toBeObject();
        });
    });

    describe('surfnet_exportSnapshot', () => {
        test('exports a snapshot object', async () => {
            const snapshot = await client.surfnet.exportSnapshot().send();
            expect(snapshot).toBeObject();
        });
    });

    describe('surfnet_setSupply', () => {
        test('sets supply without error', async () => {
            const result = await client.surfnet
                .setSupply({ total: 500_000_000_000_000 })
                .send();
            assertNullResponse(result);
        });
    });

    // resetNetwork runs last — it wipes all state
    describe('surfnet_resetNetwork', () => {
        test('returns null on success', async () => {
            const result = await client.surfnet.resetNetwork().send();
            assertNullResponse(result);
        });
    });
});
