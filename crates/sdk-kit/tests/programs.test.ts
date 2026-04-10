import { describe, test, expect, beforeAll } from 'bun:test';
import { address } from '@solana/kit';
import { testClient } from './helpers/client.ts';
import { assertNullResponse } from './helpers/assertions.ts';

let client: Awaited<typeof testClient>;

beforeAll(async () => {
    client = await testClient;
});

// Token-2022 is a BPF Upgradeable Loader program
const SRC_PROGRAM = address('TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb');
const DEST_PROGRAM = address('Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS');

describe('Program management', () => {
    describe('surfnet_cloneProgramAccount', () => {
        test('clones a program to a new address', async () => {
            const result = await client.surfnet
                .cloneProgramAccount(SRC_PROGRAM, DEST_PROGRAM)
                .send();
            assertNullResponse(result);
        });

        test('cloned program is executable', async () => {
            const info = await client.rpc
                .getAccountInfo(DEST_PROGRAM, { encoding: 'base64' })
                .send();
            expect(info.value?.executable).toBe(true);
        });
    });

    describe('surfnet_setProgramAuthority', () => {
        test('sets a new authority', async () => {
            const newAuthority = address('11111111111111111111111111111111');
            const result = await client.surfnet
                .setProgramAuthority(SRC_PROGRAM, newAuthority)
                .send();
            assertNullResponse(result);
        });
    });

    describe('surfnet_writeProgram', () => {
        // Skipped: surfpool crashes (ECONNRESET) when writing to cloned programs.
        test.skip('writes data at offset 0', async () => {
            const result = await client.surfnet
                .writeProgram(DEST_PROGRAM, 'deadbeef', 0)
                .send();
            assertNullResponse(result);
        });
    });
});
