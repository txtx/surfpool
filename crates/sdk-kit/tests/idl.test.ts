import { describe, test, expect, beforeAll } from 'bun:test';
import { address } from '@solana/kit';
import { testClient } from './helpers/client.ts';
import { assertNullResponse } from './helpers/assertions.ts';
import type { AnchorIdl } from '../src/types.ts';

let client: Awaited<typeof testClient>;

beforeAll(async () => {
    client = await testClient;
});

const TEST_PROGRAM = address('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA');

const MINIMAL_IDL: AnchorIdl = {
    address: TEST_PROGRAM,
    metadata: { name: 'test-idl', version: '0.1.0', spec: '0.1.0' },
    instructions: [],
};

describe('IDL management', () => {
    describe('surfnet_registerIdl', () => {
        test('registers an IDL without error', async () => {
            const result = await client.surfnet.registerIdl(MINIMAL_IDL).send();
            assertNullResponse(result);
        });
    });

    describe('surfnet_getActiveIdl', () => {
        test('retrieves the registered IDL', async () => {
            await client.surfnet.registerIdl(MINIMAL_IDL).send();
            const idl = await client.surfnet.getActiveIdl(TEST_PROGRAM).send();
            expect(idl).not.toBeNull();
            expect(idl?.address).toBe(TEST_PROGRAM);
        });

        test('returns null for a program with no IDL', async () => {
            const unknownProgram = address('11111111111111111111111111111111');
            const idl = await client.surfnet.getActiveIdl(unknownProgram).send();
            expect(idl).toBeNull();
        });
    });
});
