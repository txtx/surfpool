import { createEmptyClient, extendClient, type ClusterUrl, type DefaultRpcSubscriptionsChannelConfig, type MicroLamports, type TransactionSigner } from '@solana/kit';
import { createSurfnetCheatcodesRpc } from './surfnet-rpc.ts';
import { localhostRpc, rpcAirdrop, rpcGetMinimumBalance, rpcTransactionPlanExecutor, rpcTransactionPlanner } from '@solana/kit-plugin-rpc';
import { payerOrGeneratedPayer } from '@solana/kit-plugin-payer';
import { planAndSendTransactions } from '@solana/kit-plugin-instruction-plan';

export type { SurfnetCheatcodesApi } from './types.ts';
export * from './types.ts';
export { createSurfnetCheatcodesRpc } from './surfnet-rpc.ts';

const DEFAULT_ENDPOINT = 'http://127.0.0.1:8899';

/**
 * Kit plugin that adds Surfnet cheatcode capabilities to a client.
 *
 * Compose with `localhostRpc()` or `createLocalClient()` for the
 * standard Solana RPC — this plugin only adds `client.surfnet`.
 *
 * @example
 * ```ts
 * import { createEmptyClient } from '@solana/kit';
 * import { localhostRpc } from '@solana/kit-plugin-rpc';
 * import { surfpool } from '@surfpool/kit';
 *
 * const client = createEmptyClient()
 *     .use(localhostRpc())
 *     .use(surfpool());
 *
 * // Standard Solana RPC (from localhostRpc)
 * const { value: balance } = await client.rpc.getBalance(address).send();
 *
 * // Surfnet cheatcodes (from surfpool)
 * await client.surfnet.timeTravel({ absoluteEpoch: 1000 }).send();
 * ```
 */
export function surfpool(config?: { url?: string }) {
    const surfnet = createSurfnetCheatcodesRpc(config?.url ?? DEFAULT_ENDPOINT);
    return <T extends object>(client: T) => extendClient(client, { surfnet });
}

/**
 * Configuration options for RPC client factory functions.
 */
export type ClientConfig<TClusterUrl extends ClusterUrl = ClusterUrl> = {
    maxConcurrency?: number;
    payer: TransactionSigner;
    priorityFees?: MicroLamports;
    rpcSubscriptionsConfig?: DefaultRpcSubscriptionsChannelConfig<TClusterUrl>;
    skipPreflight?: boolean;
    url: TClusterUrl;
};

/**
 * Creates a batteries-included Surfpool client for local development.
 *
 * Composes localhostRpc for standard Solana RPC, surfpool for Surfnet
 * cheatcodes, and the full transaction pipeline (airdrop, payer,
 * planner, executor, plan-and-send).
 *
 * @example
 * ```ts
 * import { createSurfpoolClient } from '@surfpool/kit';
 *
 * const client = await createSurfpoolClient();
 * await client.surfnet.timeTravel({ absoluteEpoch: 1000 }).send();
 * ```
 */
export function createSurfpoolClient(
    config: Omit<ClientConfig<string>, 'payer' | 'url'> & {
        payer?: TransactionSigner;
        url?: string;
    } = {},
) {
    return createEmptyClient()
        .use(localhostRpc(config.url, config.rpcSubscriptionsConfig))
        .use(surfpool({ url: config.url }))
        .use(rpcAirdrop())
        .use(rpcGetMinimumBalance())
        .use(payerOrGeneratedPayer(config.payer))
        .use(rpcTransactionPlanner({ priorityFees: config.priorityFees }))
        .use(rpcTransactionPlanExecutor({ maxConcurrency: config.maxConcurrency, skipPreflight: config.skipPreflight }))
        .use(planAndSendTransactions());
}
