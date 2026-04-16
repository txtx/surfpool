import { describe, test, beforeAll } from 'bun:test';
import { address } from '@solana/kit';
import { testClient } from './helpers/client.ts';
import { assertNullResponse } from './helpers/assertions.ts';
import type { Scenario } from '../src/types.ts';

let client: Awaited<typeof testClient>;

beforeAll(async () => {
    client = await testClient;
});

const TEST_SCENARIO: Scenario = {
    id: 'test-scenario-1',
    name: 'Test Scenario',
    description: 'A minimal test scenario',
    overrides: [
        {
            id: 'override-1',
            templateId: 'template-1',
            values: { foo: 'bar' },
            scenarioRelativeSlot: 0,
            label: 'Test Override',
            enabled: true,
            fetchBeforeUse: false,
            account: { pubkey: address('11111111111111111111111111111111') },
        },
    ],
    tags: ['test'],
};

describe('Scenario management', () => {
    describe('surfnet_registerScenario', () => {
        test('registers a scenario without error', async () => {
            const result = await client.surfnet
                .registerScenario(TEST_SCENARIO)
                .send();
            assertNullResponse(result);
        });
    });
});
