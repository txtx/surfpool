import {
  Surfnet as SurfnetInner,
  SurfnetConfig,
  KeypairInfo,
  EpochInfoValue,
  SolAccountFunding,
} from "./internal";

export { SurfnetConfig, KeypairInfo, EpochInfoValue, SolAccountFunding } from "./internal";

/**
 * A running Surfpool instance with RPC/WS endpoints on dynamic ports.
 *
 * @example
 * ```ts
 * const surfnet = Surfnet.start();
 * console.log(surfnet.rpcUrl); // http://127.0.0.1:xxxxx
 *
 * surfnet.fundSol(address, 5_000_000_000); // 5 SOL
 * surfnet.fundToken(address, usdcMint, 1_000_000); // 1 USDC
 * ```
 */
export class Surfnet {
  private inner: SurfnetInner;

  private constructor(inner: SurfnetInner) {
    this.inner = inner;
  }

  /** Start a surfnet with default settings (offline, tx-mode blocks, 10 SOL payer). */
  static start(): Surfnet {
    return new Surfnet(SurfnetInner.start());
  }

  /** Start a surfnet with custom configuration. */
  static startWithConfig(config: SurfnetConfig): Surfnet {
    return new Surfnet(SurfnetInner.startWithConfig(config));
  }

  /** The HTTP RPC URL (e.g. "http://127.0.0.1:12345"). */
  get rpcUrl(): string {
    return this.inner.rpcUrl;
  }

  /** The WebSocket URL (e.g. "ws://127.0.0.1:12346"). */
  get wsUrl(): string {
    return this.inner.wsUrl;
  }

  /** The pre-funded payer public key as a base58 string. */
  get payer(): string {
    return this.inner.payer;
  }

  /** The pre-funded payer secret key as a 64-byte Uint8Array. */
  get payerSecretKey(): Uint8Array {
    return Uint8Array.from(this.inner.payerSecretKey);
  }

  /** Fund a SOL account with lamports. */
  fundSol(address: string, lamports: number): void {
    this.inner.fundSol(address, lamports);
  }

  /** Fund multiple SOL accounts with explicit lamport balances. */
  fundSolMany(accounts: SolAccountFunding[]): void {
    this.inner.fundSolMany(accounts);
  }

  /**
   * Fund a token account (creates the ATA if needed).
   * Uses spl_token program by default. Pass tokenProgram for Token-2022.
   */
  fundToken(
    owner: string,
    mint: string,
    amount: number,
    tokenProgram?: string
  ): void {
    this.inner.fundToken(owner, mint, amount, tokenProgram ?? null);
  }

  /** Set the token balance for a wallet/mint pair. */
  setTokenBalance(
    owner: string,
    mint: string,
    amount: number,
    tokenProgram?: string
  ): void {
    this.inner.setTokenBalance(owner, mint, amount, tokenProgram ?? null);
  }

  /** Fund multiple wallets with the same token and amount. */
  fundTokenMany(
    owners: string[],
    mint: string,
    amount: number,
    tokenProgram?: string
  ): void {
    this.inner.fundTokenMany(owners, mint, amount, tokenProgram ?? null);
  }

  /** Set arbitrary account data. */
  setAccount(
    address: string,
    lamports: number,
    data: Uint8Array,
    owner: string
  ): void {
    this.inner.setAccount(address, lamports, Array.from(data), owner);
  }

  /** Move Surfnet time forward to an absolute epoch. */
  timeTravelToEpoch(epoch: number): EpochInfoValue {
    return this.inner.timeTravelToEpoch(epoch);
  }

  /** Move Surfnet time forward to an absolute slot. */
  timeTravelToSlot(slot: number): EpochInfoValue {
    return this.inner.timeTravelToSlot(slot);
  }

  /** Move Surfnet time forward to an absolute Unix timestamp in milliseconds. */
  timeTravelToTimestamp(timestamp: number): EpochInfoValue {
    return this.inner.timeTravelToTimestamp(timestamp);
  }

  /** Deploy a program by discovering local Anchor/Agave artifacts. */
  deployProgram(programName: string): void {
    this.inner.deployProgram(programName);
  }

  /** Get the associated token address for a wallet/mint pair. */
  getAta(owner: string, mint: string, tokenProgram?: string): string {
    return this.inner.getAta(owner, mint, tokenProgram ?? null);
  }

  /** Generate a new random keypair. */
  static newKeypair(): KeypairInfo {
    return SurfnetInner.newKeypair();
  }
}
