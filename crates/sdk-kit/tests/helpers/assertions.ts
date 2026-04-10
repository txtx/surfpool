import { expect } from 'bun:test';
import type { ClockState } from '../../src/types.ts';

export function assertClockState(state: unknown): asserts state is ClockState {
    expect(state).toBeObject();
    const s = state as Record<string, unknown>;
    expect(typeof s.absoluteSlot).toBe('number');
    expect(typeof s.blockHeight).toBe('number');
    expect(typeof s.epoch).toBe('number');
    expect(typeof s.slotIndex).toBe('number');
    expect(typeof s.slotsInEpoch).toBe('number');
    expect(typeof s.transactionCount).toBe('number');
    expect(s.absoluteSlot as number).toBeGreaterThanOrEqual(0);
    expect(s.epoch as number).toBeGreaterThanOrEqual(0);
}

export function assertNullResponse(value: unknown): void {
    expect(value).toBeNull();
}
