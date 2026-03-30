import { describe, test, expect, beforeAll, afterAll } from 'bun:test';
import { testClient } from './helpers/client.ts';
import { assertClockState } from './helpers/assertions.ts';

let client: Awaited<typeof testClient>;

beforeAll(async () => {
    client = await testClient;
});

async function currentEpoch(): Promise<number> {
    const info = await client.rpc.getEpochInfo().send();
    return Number(info.epoch);
}

describe('Clock control', () => {
    afterAll(async () => {
        try {
            await client.surfnet.resumeClock().send();
        } catch {
            // ignore if already running
        }
    });

    describe('surfnet_timeTravel', () => {
        test('travels forward to a future epoch', async () => {
            const targetEpoch = (await currentEpoch()) + 100;
            const result = await client.surfnet.timeTravel({ absoluteEpoch: targetEpoch }).send();

            assertClockState(result);
            expect(result.epoch).toBe(targetEpoch);
        });

        test('travels to an absolute slot', async () => {
            const info = await client.rpc.getEpochInfo().send();
            const targetSlot = Number(info.absoluteSlot) + 100_000;
            const result = await client.surfnet.timeTravel({ absoluteSlot: targetSlot }).send();

            assertClockState(result);
            expect(result.absoluteSlot).toBe(targetSlot);
        });

        test('clock state is reflected by getEpochInfo', async () => {
            const target = (await currentEpoch()) + 50;
            await client.surfnet.timeTravel({ absoluteEpoch: target }).send();

            const epochInfo = await client.rpc.getEpochInfo().send();
            expect(Number(epochInfo.epoch)).toBe(target);
        });

        test('throws when traveling to a past epoch', async () => {
            await expect(
                client.surfnet.timeTravel({ absoluteEpoch: 0 }).send(),
            ).rejects.toThrow();
        });
    });

    describe('surfnet_pauseClock', () => {
        test('returns clock state on pause', async () => {
            const result = await client.surfnet.pauseClock().send();
            assertClockState(result);
        });

        test('clock does not advance while paused', async () => {
            await client.surfnet.pauseClock().send();
            const before = await client.rpc.getSlot().send();
            await Bun.sleep(500);
            const after = await client.rpc.getSlot().send();
            expect(after).toBe(before);
            await client.surfnet.resumeClock().send();
        });
    });

    describe('surfnet_resumeClock', () => {
        test('returns clock state on resume', async () => {
            await client.surfnet.pauseClock().send();
            const result = await client.surfnet.resumeClock().send();
            assertClockState(result);
        });
    });
});
