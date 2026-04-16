const SURFPOOL_URL = process.env.SURFPOOL_URL ?? 'http://127.0.0.1:8899';

export async function probeHealth(): Promise<void> {
    try {
        const res = await fetch(SURFPOOL_URL, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'getHealth' }),
            signal: AbortSignal.timeout(3000),
        });
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
    } catch (err) {
        throw new Error(
            `Surfpool is not reachable at ${SURFPOOL_URL}. Start it before running tests.\n${err}`,
        );
    }
}
