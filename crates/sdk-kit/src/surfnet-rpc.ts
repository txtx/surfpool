import {
    type Rpc,
    createDefaultRpcTransport,
    createRpc,
    createJsonRpcApi,
} from '@solana/rpc';
import type { RpcResponse, RpcResponseData } from '@solana/rpc-spec-types';
import type { SurfnetCheatcodesApi } from './types.ts';

const DEFAULT_SURFNET_ENDPOINT = 'http://127.0.0.1:8899';

/**
 * Creates a Surfnet cheatcodes RPC client with clean method names.
 *
 * Method names have the `surfnet_` prefix stripped — e.g. `timeTravel()`
 * instead of `surfnet_timeTravel()`. The prefix is added back on the wire
 * via the request transformer.
 */
export function createSurfnetCheatcodesRpc(
    endpoint: string = DEFAULT_SURFNET_ENDPOINT,
): Rpc<SurfnetCheatcodesApi> {
    const api = createJsonRpcApi<SurfnetCheatcodesApi>({
        requestTransformer: (request) => ({
            ...request,
            methodName: `surfnet_${request.methodName}`,
        }),
        responseTransformer: (response: RpcResponse) => {
            const r = response as RpcResponseData<unknown>;
            if ('error' in r) {
                throw new Error(
                    `Surfnet RPC error ${r.error.code}: ${r.error.message}${r.error.data ? ` — ${r.error.data}` : ''}`,
                );
            }
            const result = r.result;
            // Surfnet wraps cheatcode responses in { context, value }.
            // Unwrap to return the inner value directly.
            if (result != null && typeof result === 'object' && 'value' in result) {
                return (result as { value: unknown }).value;
            }
            return result;
        },
    });
    const transport = createDefaultRpcTransport({ url: endpoint });
    return createRpc({ api, transport });
}
