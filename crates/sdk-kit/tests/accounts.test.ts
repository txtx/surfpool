import { describe, test, expect, beforeAll } from 'bun:test';
import { address, lamports } from '@solana/kit';
import { testClient } from './helpers/client.ts';
import { assertNullResponse } from './helpers/assertions.ts';

let client: Awaited<typeof testClient>;

beforeAll(async () => {
    client = await testClient;
});

// A burn address we can safely mutate in tests
const TEST_PUBKEY = address('1nc1nerator11111111111111111111111111111111');

describe('Account manipulation', () => {
    describe('surfnet_setAccount', () => {
        test('sets lamports on an account', async () => {
            const result = await client.surfnet
                .setAccount(TEST_PUBKEY, { lamports: 5_000_000_000 })
                .send();
            assertNullResponse(result);
        });

        test('getBalance reflects new lamports', async () => {
            await client.surfnet
                .setAccount(TEST_PUBKEY, { lamports: 3_000_000_000 })
                .send();
            const { value: balance } = await client.rpc.getBalance(TEST_PUBKEY).send();
            expect(balance).toBe(lamports(3_000_000_000n));
        });

        test('sets data on an account', async () => {
            const result = await client.surfnet
                .setAccount(TEST_PUBKEY, {
                    lamports: 1_000_000_000,
                    data: 'deadbeef',
                })
                .send();
            assertNullResponse(result);
        });
    });

    describe('surfnet_resetAccount', () => {
        beforeAll(async () => {
            await client.surfnet
                .setAccount(TEST_PUBKEY, { lamports: 5_000_000_000 })
                .send();
        });

        test('resets account state', async () => {
            const result = await client.surfnet.resetAccount(TEST_PUBKEY).send();
            assertNullResponse(result);
        });
    });

    describe('surfnet_setTokenAccount', () => {
        const USDC_MINT = address('EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v');
        const TOKEN_OWNER = address('11111111111111111111111111111111');

        test('sets a token account balance', async () => {
            const result = await client.surfnet
                .setTokenAccount(TOKEN_OWNER, USDC_MINT, {
                    amount: 1_000_000,
                })
                .send();
            assertNullResponse(result);
        });
    });

    describe('surfnet_streamAccount / surfnet_getStreamedAccounts', () => {
        test('registers an account for streaming', async () => {
            const result = await client.surfnet.streamAccount(TEST_PUBKEY).send();
            assertNullResponse(result);
        });

        test('getStreamedAccounts returns registered accounts', async () => {
            await client.surfnet.streamAccount(TEST_PUBKEY).send();
            const result = await client.surfnet.getStreamedAccounts().send();
            expect(result.accounts).toBeArray();
        });
    });
});
