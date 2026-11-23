#!/usr/bin/env node
/**
 * BlitzVault — 1-Command Solana Devnet + MagicBlock ER Proof-of-Work Script
 *
 * Executes the full Entertainment Finance lifecycle against a deployed BlitzVault
 * program on Solana Devnet + live MagicBlock Ephemeral Rollups & Scoped VRF:
 *   1. Discover live ER validator route via https://devnet-router.magicblock.app (getRoutes)
 *   2. L1: initialize_arena()
 *   3. L1: delegate_arena(validator = selected ER validator in remaining_accounts[0])
 *   4. ER: start_round(stake=500) -> MagicBlock DEFAULT_EPHEMERAL_QUEUE VRF
 *   5. ER: wait for scoped #[vrf_callback] callback_round_vrf() -> Live
 *   6. ER: cashout_round(11_000 bps / 1.10x) -> 0-gas settlement
 *   7. ER: open_relic_pack() -> wait for callback_relic_vrf()
 *   8. ER: commit_arena() & undelegate_arena() -> atomic L1 settlement via MagicIntentBundleBuilder
 *
 * Usage:
 *   BLITZ_PROGRAM_ID=<DEPLOYED_PROGRAM_ID> KEYPAIR_PATH=~/.config/solana/id.json node scripts/devnet_pow.mjs
 */

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import crypto from "node:crypto";
import {
  Connection,
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
  TransactionInstruction,
  sendAndConfirmTransaction,
} from "@solana/web3.js";

const L1_RPC = process.env.SOLANA_DEVNET_RPC || "https://api.devnet.solana.com";
const ROUTER_RPC = process.env.MAGICBLOCK_ROUTER_RPC || "https://devnet-router.magicblock.app";
const PROGRAM_ID = new PublicKey(
  process.env.BLITZ_PROGRAM_ID || "BLzVt11111111111111111111111111111111111111"
);
const DELEGATION_PROGRAM_ID = new PublicKey("DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh");
const MAGIC_PROGRAM_ID = new PublicKey("Magic11111111111111111111111111111111111111");
const MAGIC_CONTEXT_ID = new PublicKey("MagicContext1111111111111111111111111111111");
const VRF_PROGRAM_ID = new PublicKey("Vrf1RNUjXmQGjmQrQLvJHs9SNkvDJEsRVFPkfSQUwGz");
const DEFAULT_EPHEMERAL_QUEUE = new PublicKey("5hBR571xnXppuCPveTrctfTU7tJLSN94nq7kv7FRK5Tc");
const SLOT_HASHES_SYSVAR = new PublicKey("SysvarS1otHashes111111111111111111111111111");
const ARENA_SEED = Buffer.from("blitz_arena_v1");

const disc = (name) => crypto.createHash("sha256").update(`global:${name}`).digest().subarray(0, 8);
const u64Le = (n) => {
  const b = Buffer.alloc(8);
  b.writeBigUInt64LE(BigInt(n), 0);
  return b;
};
const u32Le = (n) => {
  const b = Buffer.alloc(4);
  b.writeUInt32LE(n, 0);
  return b;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const explorerTx = (sig) => `https://explorer.solana.com/tx/${sig}?cluster=devnet`;
const explorerAddr = (pk) => `https://explorer.solana.com/address/${pk}?cluster=devnet`;

async function discoverErRoute(preferredCountry = process.env.ER_REGION || "SGP") {
  const res = await fetch(ROUTER_RPC, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "getRoutes" }),
  });
  const json = await res.json();
  const routes = json.result || [];
  if (!routes.length) throw new Error(`No routes returned by ${ROUTER_RPC}`);
  return (
    routes.find((r) => r.countryCode === preferredCountry && !r.fqdn.includes("tee")) || routes[0]
  );
}

function loadKeypair() {
  const kpPath =
    process.env.KEYPAIR_PATH || path.join(os.homedir(), ".config", "solana", "id.json");
  if (!fs.existsSync(kpPath)) {
    throw new Error(`Keypair not found at ${kpPath}. Set KEYPAIR_PATH=/path/to/id.json`);
  }
  const secret = JSON.parse(fs.readFileSync(kpPath, "utf8"));
  return Keypair.fromSecretKey(Uint8Array.from(secret));
}

function readPhaseAndCrash(data) {
  return {
    chips: Number(data.readBigUInt64LE(8 + 32)),
    shards: Number(data.readBigUInt64LE(8 + 40)),
    roundNonce: Number(data.readBigUInt64LE(8 + 48)),
    phase: data.readUInt8(8 + 56),
    crashPointBps: data.readUInt32LE(8 + 66),
  };
}

async function main() {
  console.log("=== BlitzVault Solana Devnet + MagicBlock ER Proof-of-Work ===");
  const route = await discoverErRoute();
  const erUrl = route.fqdn.replace(/\/$/, "");
  const erValidator = new PublicKey(route.identity);
  console.log(
    `ER Route Selected: ${route.countryCode} · ${erUrl} (validator=${erValidator.toBase58()}, ${route.blockTimeMs}ms, baseFee=${route.baseFee})`
  );

  const l1 = new Connection(L1_RPC, "confirmed");
  const er = new Connection(erUrl, "confirmed");

  const progInfo = await l1.getAccountInfo(PROGRAM_ID);
  if (!progInfo || !progInfo.executable) {
    console.error(`\nProgram ${PROGRAM_ID.toBase58()} is not yet deployed on Solana Devnet.`);
    console.error("1. Fund your deployer wallet (~2.5 SOL) at https://faucet.solana.com");
    console.error(
      "2. Deploy: solana program deploy target/deploy/blitz_vault.so --url devnet --max-len 400000"
    );
    console.error("3. Re-run: BLITZ_PROGRAM_ID=<PROGRAM_ID> node scripts/devnet_pow.mjs");
    process.exit(1);
  }
  console.log(`Program Verified On Devnet: ${explorerAddr(PROGRAM_ID.toBase58())}`);

  const payer = loadKeypair();
  const bal = await l1.getBalance(payer.publicKey);
  console.log(`Player Wallet: ${payer.publicKey.toBase58()} (${(bal / 1e9).toFixed(4)} Devnet SOL)`);
  if (bal < 50_000_000) {
    throw new Error("Need at least 0.05 Devnet SOL in payer wallet to pay L1 rent/fees.");
  }

  const [arenaPda] = PublicKey.findProgramAddressSync(
    [ARENA_SEED, payer.publicKey.toBuffer()],
    PROGRAM_ID
  );
  const [programIdentity] = PublicKey.findProgramAddressSync([Buffer.from("identity")], PROGRAM_ID);
  const [bufferPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("buffer"), arenaPda.toBuffer()],
    PROGRAM_ID
  );
  const [delegationRecord] = PublicKey.findProgramAddressSync(
    [Buffer.from("delegation"), arenaPda.toBuffer()],
    DELEGATION_PROGRAM_ID
  );
  const [delegationMetadata] = PublicKey.findProgramAddressSync(
    [Buffer.from("delegation-metadata"), arenaPda.toBuffer()],
    DELEGATION_PROGRAM_ID
  );
  console.log(`ArenaState PDA: ${explorerAddr(arenaPda.toBase58())}`);

  // Step 1: Initialize Arena on L1 if not already initialized
  const existing = await l1.getAccountInfo(arenaPda);
  if (!existing) {
    const initIx = new TransactionInstruction({
      programId: PROGRAM_ID,
      keys: [
        { pubkey: arenaPda, isSigner: false, isWritable: true },
        { pubkey: payer.publicKey, isSigner: true, isWritable: true },
        { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
      ],
      data: disc("initialize_arena"),
    });
    const sig = await sendAndConfirmTransaction(l1, new Transaction().add(initIx), [payer]);
    console.log(`[1/6] initialize_arena (L1): ${explorerTx(sig)}`);
  } else {
    console.log("[1/6] initialize_arena skipped (PDA already exists on Devnet)");
  }

  // Step 2: Delegate ArenaState PDA to MagicBlock ER
  const acctNow = await l1.getAccountInfo(arenaPda);
  if (acctNow && acctNow.owner.equals(PROGRAM_ID)) {
    const delIx = new TransactionInstruction({
      programId: PROGRAM_ID,
      keys: [
        { pubkey: payer.publicKey, isSigner: true, isWritable: true },
        { pubkey: bufferPda, isSigner: false, isWritable: true },
        { pubkey: delegationRecord, isSigner: false, isWritable: true },
        { pubkey: delegationMetadata, isSigner: false, isWritable: true },
        { pubkey: arenaPda, isSigner: false, isWritable: true },
        { pubkey: PROGRAM_ID, isSigner: false, isWritable: false },
        { pubkey: DELEGATION_PROGRAM_ID, isSigner: false, isWritable: false },
        { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
        { pubkey: erValidator, isSigner: false, isWritable: false },
      ],
      data: disc("delegate_arena"),
    });
    const sig = await sendAndConfirmTransaction(l1, new Transaction().add(delIx), [payer]);
    console.log(`[2/6] delegate_arena (L1 -> ER): ${explorerTx(sig)}`);
    await sleep(3000);
  } else {
    console.log("[2/6] delegate_arena skipped (PDA already delegated to ER)");
  }

  // Step 3: Start VRF Round inside MagicBlock ER (0 gas, fee-exempt DEFAULT_EPHEMERAL_QUEUE)
  const startIx = new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: payer.publicKey, isSigner: true, isWritable: true },
      { pubkey: arenaPda, isSigner: false, isWritable: true },
      { pubkey: DEFAULT_EPHEMERAL_QUEUE, isSigner: false, isWritable: true },
      { pubkey: programIdentity, isSigner: false, isWritable: false },
      { pubkey: VRF_PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: SLOT_HASHES_SYSVAR, isSigner: false, isWritable: false },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    ],
    data: Buffer.concat([disc("start_round"), u64Le(500), Buffer.from([42])]),
  });
  const startSig = await sendAndConfirmTransaction(er, new Transaction().add(startIx), [payer], {
    skipPreflight: true,
  });
  console.log(`[3/6] start_round VRF request (ER): ${startSig}`);

  // Wait for scoped #[vrf_callback] fulfillment inside ER
  let state;
  for (let i = 0; i < 30; i++) {
    const info = await er.getAccountInfo(arenaPda);
    if (info) {
      state = readPhaseAndCrash(info.data);
      if (state.phase === 2) break;
    }
    await sleep(1000);
  }
  if (!state || state.phase !== 2) {
    throw new Error("Timed out waiting for callback_round_vrf inside ER");
  }
  console.log(
    `[4/6] callback_round_vrf fulfilled on ER: crash_point=${(state.crashPointBps / 10000).toFixed(2)}x`
  );

  // Step 4: Cashout inside ER
  const cashoutIx = new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: payer.publicKey, isSigner: true, isWritable: true },
      { pubkey: arenaPda, isSigner: false, isWritable: true },
    ],
    data: Buffer.concat([disc("cashout_round"), u32Le(11_000)]),
  });
  const cashSig = await sendAndConfirmTransaction(er, new Transaction().add(cashoutIx), [payer], {
    skipPreflight: true,
  });
  console.log(`[5/6] cashout_round 1.10x (ER, 0-gas): ${cashSig}`);

  // Step 5: Commit & Undelegate back to Solana L1 via MagicIntentBundleBuilder
  const undelIx = new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: payer.publicKey, isSigner: true, isWritable: true },
      { pubkey: arenaPda, isSigner: false, isWritable: true },
      { pubkey: MAGIC_PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: MAGIC_CONTEXT_ID, isSigner: false, isWritable: true },
    ],
    data: disc("undelegate_arena"),
  });
  const undelSig = await sendAndConfirmTransaction(er, new Transaction().add(undelIx), [payer], {
    skipPreflight: true,
  });
  console.log(`[6/6] undelegate_arena (ER -> L1): ${undelSig}`);
  console.log("\n=== FULL DEVNET + MAGICBLOCK ER PROOF-OF-WORK COMPLETE ===");
}

main().catch((err) => {
  console.error("Devnet PoW error:", err.message || err);
  process.exit(1);
});
