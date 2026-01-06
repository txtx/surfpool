/**
 * Escrow lifecycle test - create, release, dispute, expire.
 * Uses Surfpool to skip time instead of waiting.
 */

import {
  Connection,
  Keypair,
  PublicKey,
  Transaction,
  SystemProgram,
  LAMPORTS_PER_SOL,
} from "@solana/web3.js";

const SURFPOOL_ENDPOINT = process.env.SURFPOOL_URL || "http://localhost:8899";

// Simulated escrow state (replace with your program's structure)
interface Escrow {
  pda: PublicKey;
  agent: PublicKey;
  provider: PublicKey;
  amount: number;
  createdAt: number;
  lockSeconds: number;
  status: "active" | "released" | "disputed" | "expired";
}

// In-memory escrow storage for demo
const escrows = new Map<string, Escrow>();

async function main() {
  const connection = new Connection(SURFPOOL_ENDPOINT, "confirmed");

  // Generate test accounts
  const agent = Keypair.generate();
  const provider = Keypair.generate();

  console.log("=== Payment Escrow Lifecycle Test ===\n");
  console.log(`Agent:    ${agent.publicKey.toBase58().slice(0, 8)}...`);
  console.log(`Provider: ${provider.publicKey.toBase58().slice(0, 8)}...`);

  // Fund agent account
  await fundAccount(connection, agent.publicKey, 10 * LAMPORTS_PER_SOL);
  console.log("\nAgent funded with 10 SOL");

  // ========================================================================
  // Phase 1: Create Escrow
  // ========================================================================
  console.log("\n--- Phase 1: Create Escrow ---");

  const escrow = await createEscrow(connection, agent, provider.publicKey, {
    amount: 1 * LAMPORTS_PER_SOL,
    lockSeconds: 3600, // 1 hour lock
  });

  console.log(`Escrow created: ${escrow.pda.toBase58().slice(0, 8)}...`);
  console.log(`Amount: ${escrow.amount / LAMPORTS_PER_SOL} SOL`);
  console.log(`Lock: ${escrow.lockSeconds} seconds`);
  console.log(`Status: ${escrow.status}`);

  // ========================================================================
  // Phase 2: Attempt Early Release (Should Fail)
  // ========================================================================
  console.log("\n--- Phase 2: Early Release Attempt ---");

  // Advance 30 minutes (still within lock)
  await advanceTime(connection, 1800);
  console.log("Advanced 30 minutes");

  const earlyRelease = await attemptRelease(connection, escrow, provider);
  console.log(`Early release result: ${earlyRelease.success ? "SUCCESS" : "FAILED"}`);
  if (!earlyRelease.success) {
    console.log(`Reason: ${earlyRelease.error}`);
  }

  // ========================================================================
  // Phase 3: Advance Past Lock and Release
  // ========================================================================
  console.log("\n--- Phase 3: Release After Lock ---");

  // Advance past the lock period
  await advanceTime(connection, 3600);
  console.log("Advanced 1 hour (past lock)");

  const release = await attemptRelease(connection, escrow, provider);
  console.log(`Release result: ${release.success ? "SUCCESS" : "FAILED"}`);

  if (release.success) {
    const providerBalance = await connection.getBalance(provider.publicKey);
    console.log(`Provider balance: ${providerBalance / LAMPORTS_PER_SOL} SOL`);
  }

  // ========================================================================
  // Phase 4: Test Dispute Flow (Fresh Escrow)
  // ========================================================================
  console.log("\n--- Phase 4: Dispute Flow ---");

  // Create new escrow for dispute testing
  const escrow2 = await createEscrow(connection, agent, provider.publicKey, {
    amount: 0.5 * LAMPORTS_PER_SOL,
    lockSeconds: 7200, // 2 hour lock
  });

  console.log(`New escrow: ${escrow2.pda.toBase58().slice(0, 8)}...`);

  // Agent disputes (claiming poor service)
  const dispute = await initiateDispute(connection, escrow2, agent);
  console.log(`Dispute initiated: ${dispute.success}`);

  // Simulate oracle resolution (50% refund)
  const resolution = await resolveDispute(connection, escrow2, 50);
  console.log(`Resolution: ${resolution.agentRefund}% to agent, ${resolution.providerPayment}% to provider`);

  // ========================================================================
  // Phase 5: Test Expiration (Fresh Escrow)
  // ========================================================================
  console.log("\n--- Phase 5: Expiration Flow ---");

  // Create escrow with short expiry
  const escrow3 = await createEscrow(connection, agent, provider.publicKey, {
    amount: 0.25 * LAMPORTS_PER_SOL,
    lockSeconds: 300, // 5 minute lock
  });

  console.log(`Short-lived escrow: ${escrow3.pda.toBase58().slice(0, 8)}...`);

  // Advance past expiry + grace period (assume 7 day grace)
  const gracePeriod = 7 * 24 * 3600;
  await advanceTime(connection, escrow3.lockSeconds + gracePeriod + 1);
  console.log("Advanced past expiry + grace period");

  // Claim expired escrow (permissionless)
  const expiryClaim = await claimExpired(connection, escrow3);
  console.log(`Expiry claim: ${expiryClaim.success ? "SUCCESS" : "FAILED"}`);
  console.log(`Funds returned to: ${expiryClaim.recipient}`);

  console.log("\n=== Lifecycle Test Complete ===");
}

// ============================================================================
// Helper Functions (Replace with your actual program interactions)
// ============================================================================

async function fundAccount(
  connection: Connection,
  account: PublicKey,
  amount: number
): Promise<void> {
  // In Surfpool, use the airdrop or setBalance cheatcode
  const signature = await connection.requestAirdrop(account, amount);
  await connection.confirmTransaction(signature);
}

async function advanceTime(connection: Connection, seconds: number): Promise<void> {
  // Call Surfpool's time advancement RPC
  // This is a simplified version - actual implementation uses surfpool cheatcodes
  const slotsToAdvance = Math.ceil(seconds / 0.4); // ~400ms per slot

  // Surfpool RPC call to advance slots
  await (connection as any)._rpcRequest("surfnet_advanceSlots", [slotsToAdvance]);
}

async function createEscrow(
  connection: Connection,
  agent: Keypair,
  provider: PublicKey,
  params: { amount: number; lockSeconds: number }
): Promise<Escrow> {
  // Simulate escrow creation
  // In real implementation, this would call your Anchor program

  const slot = await connection.getSlot();
  const blockTime = await connection.getBlockTime(slot) || Math.floor(Date.now() / 1000);

  const escrow: Escrow = {
    pda: Keypair.generate().publicKey, // Would be PDA derivation
    agent: agent.publicKey,
    provider,
    amount: params.amount,
    createdAt: blockTime,
    lockSeconds: params.lockSeconds,
    status: "active",
  };

  escrows.set(escrow.pda.toBase58(), escrow);
  return escrow;
}

async function attemptRelease(
  connection: Connection,
  escrow: Escrow,
  releaser: Keypair
): Promise<{ success: boolean; error?: string }> {
  const slot = await connection.getSlot();
  const blockTime = await connection.getBlockTime(slot) || Math.floor(Date.now() / 1000);

  const stored = escrows.get(escrow.pda.toBase58());
  if (!stored || stored.status !== "active") {
    return { success: false, error: "Escrow not active" };
  }

  const unlockTime = stored.createdAt + stored.lockSeconds;

  if (blockTime < unlockTime) {
    return { success: false, error: `Locked until ${new Date(unlockTime * 1000).toISOString()}` };
  }

  stored.status = "released";
  return { success: true };
}

async function initiateDispute(
  connection: Connection,
  escrow: Escrow,
  agent: Keypair
): Promise<{ success: boolean }> {
  const stored = escrows.get(escrow.pda.toBase58());
  if (!stored || stored.status !== "active") {
    return { success: false };
  }

  stored.status = "disputed";
  return { success: true };
}

async function resolveDispute(
  connection: Connection,
  escrow: Escrow,
  qualityScore: number
): Promise<{ agentRefund: number; providerPayment: number }> {
  // Quality-based settlement
  let agentRefund: number;

  if (qualityScore >= 80) {
    agentRefund = 0;
  } else if (qualityScore >= 50) {
    agentRefund = Math.round(((80 - qualityScore) / 80) * 100);
  } else {
    agentRefund = 100;
  }

  const stored = escrows.get(escrow.pda.toBase58());
  if (stored) {
    stored.status = "released";
  }

  return {
    agentRefund,
    providerPayment: 100 - agentRefund,
  };
}

async function claimExpired(
  connection: Connection,
  escrow: Escrow
): Promise<{ success: boolean; recipient: string }> {
  const stored = escrows.get(escrow.pda.toBase58());
  if (!stored) {
    return { success: false, recipient: "" };
  }

  stored.status = "expired";

  // Expired escrow returns funds to agent
  return {
    success: true,
    recipient: "agent",
  };
}

main().catch(console.error);
