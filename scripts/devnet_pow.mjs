#!/usr/bin/env node
/**
 * BlitzVault — 1-Command Solana Devnet + MagicBlock ER Proof-of-Work Script
 *
 * Executes the full Entertainment Finance lifecycle against the deployed BlitzVault
 * program on Solana Devnet + live MagicBlock Ephemeral Rollups & Scoped VRF:
 *   1. Discover live ER validator route via https://devnet-router.magicblock.app (getRoutes)
 *   2. L1: initialize_arena() -> 148-byte ArenaState PDA (10,000 chips, 150 shards)
 *   3. L1: delegate_arena(validator = selected ER validator in remaining_accounts[0])
 *   4. ER: start_round(stake=500) -> MagicBlock DEFAULT_EPHEMERAL_QUEUE VRF
 *   5. ER: wait for scoped #[vrf_callback] callback_round_vrf() -> Live
 *   6. ER: cashout_round(11_000 bps / 1.10x) -> 0-gas settlement
 *   7. ER: open_relic_pack() -> burn 100 shards -> wait for callback_relic_vrf() -> Relic minted
 *   8. ER: commit_arena() -> checkpoint to L1 via MagicIntentBundleBuilder
 *   9. ER: undelegate_arena() -> atomic commit & undelegate back to Solana L1 + verify L1 state
 *
 * Usage:
 *   BLITZ_PROGRAM_ID=J22SmyEnav4VNywHCBacqWbN5ZYGzJY5Xegiy54xLfqU KEYPAIR_PATH=~/.config/solana/id.json node scripts/devnet_pow.mjs
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
  process.env.BLITZ_PROGRAM_ID || "J22SmyEnav4VNywHCBacqWbN5ZYGzJY5Xegiy54xLfqU"
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
    routes.find((r) => r.countryCode === preferredCountry && !r.fqdn.includes("tee")) ||
    routes.find((r) => !r.fqdn.includes("tee")) ||
    routes[0]
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

function parseArenaState(data) {
  let off = 8;
  const player = new PublicKey(data.subarray(off, off + 32)).toBase58(); off += 32;
  const chips = Number(data.readBigUInt64LE(off)); off += 8;
  const shards = Number(data.readBigUInt64LE(off)); off += 8;
  const roundNonce = Number(data.readBigUInt64LE(off)); off += 8;
  const phase = data.readUInt8(off); off += 1;
  const pendingVrfKind = data.readUInt8(off); off += 1;
  const activeStake = Number(data.readBigUInt64LE(off)); off += 8;
  const crashPointBps = data.readUInt32LE(off); off += 4;
  const lastCashoutBps = data.readUInt32LE(off); off += 4;
  const lastVrfRequestTs = Number(data.readBigInt64LE(off)); off += 8;
  const winStreak = data.readUInt16LE(off); off += 2;
  const bestStreak = data.readUInt16LE(off); off += 2;
  const roundsPlayed = data.readUInt32LE(off); off += 4;
  const totalPnl = Number(data.readBigInt64LE(off)); off += 8;
  const relicsCommon = data.readUInt16LE(off); off += 2;
  const relicsRare = data.readUInt16LE(off); off += 2;
  const relicsEpic = data.readUInt16LE(off); off += 2;
  const relicsMythic = data.readUInt16LE(off); off += 2;
  const lastRelicTier = data.readUInt8(off); off += 1;
  const lastVrfHash = data.subarray(off, off + 32).toString("hex"); off += 32;
  const bump = data.readUInt8(off); off += 1;
  return {
    player,
    chips,
    shards,
    roundNonce,
    phase,
    pendingVrfKind,
    activeStake,
    crashPointBps,
    lastCashoutBps,
    lastVrfRequestTs,
    winStreak,
    bestStreak,
    roundsPlayed,
    totalPnl,
    relicsCommon,
    relicsRare,
    relicsEpic,
    relicsMythic,
    totalRelics: relicsCommon + relicsRare + relicsEpic + relicsMythic,
    lastRelicTier,
    lastVrfHash,
    bump,
  };
}

async function sendErTx(er, ix, payer) {
  const { blockhash, lastValidBlockHeight } = await er.getLatestBlockhash("confirmed");
  const tx = new Transaction({ recentBlockhash: blockhash, feePayer: payer.publicKey }).add(ix);
  tx.sign(payer);
  const sig = await er.sendRawTransaction(tx.serialize(), { skipPreflight: true });
  await er.confirmTransaction({ signature: sig, blockhash, lastValidBlockHeight }, "confirmed");
  return sig;
}

async function main() {
  console.log("=== BlitzVault Solana Devnet + MagicBlock ER Proof-of-Work ===");
  const report = {
    timestampUtc: new Date().toISOString(),
    programId: PROGRAM_ID.toBase58(),
    programExplorer: explorerAddr(PROGRAM_ID.toBase58()),
    l1Rpc: L1_RPC,
    routerRpc: ROUTER_RPC,
    txs: {},
  };

  const route = await discoverErRoute();
  const erUrl = route.fqdn.replace(/\/$/, "");
  const erValidator = new PublicKey(route.identity);
  report.erRoute = {
    countryCode: route.countryCode,
    fqdn: erUrl,
    validator: erValidator.toBase58(),
    blockTimeMs: route.blockTimeMs,
    baseFee: route.baseFee,
  };
  console.log(
    `ER Route Selected: ${route.countryCode} · ${erUrl} (validator=${erValidator.toBase58()}, ${route.blockTimeMs}ms, baseFee=${route.baseFee})`
  );

  const l1 = new Connection(L1_RPC, "confirmed");
  const er = new Connection(erUrl, "confirmed");

  const progInfo = await l1.getAccountInfo(PROGRAM_ID);
  if (!progInfo || !progInfo.executable) {
    throw new Error(`Program ${PROGRAM_ID.toBase58()} is not deployed/executable on Solana Devnet.`);
  }
  console.log(`Program Verified On Devnet: ${explorerAddr(PROGRAM_ID.toBase58())}`);

  const payer = loadKeypair();
  const bal = await l1.getBalance(payer.publicKey);
  report.player = payer.publicKey.toBase58();
  report.balanceBeforeLamports = bal;
  console.log(`Player Wallet: ${payer.publicKey.toBase58()} (${(bal / 1e9).toFixed(6)} Devnet SOL)`);
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
  report.arenaPda = arenaPda.toBase58();
  report.arenaExplorer = explorerAddr(arenaPda.toBase58());
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
    report.txs.initialize_arena = { layer: "L1", sig, explorer: explorerTx(sig) };
    console.log(`[1/8] initialize_arena (L1): ${explorerTx(sig)}`);
  } else {
    console.log("[1/8] initialize_arena skipped (PDA already exists on Devnet)");
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
    report.txs.delegate_arena = { layer: "L1", sig, explorer: explorerTx(sig) };
    console.log(`[2/8] delegate_arena (L1 -> ER): ${explorerTx(sig)}`);

    // Wait for ER validator to ingest the delegated ArenaState PDA
    for (let i = 0; i < 20; i++) {
      await sleep(1000);
      const erInfo = await er.getAccountInfo(arenaPda);
      if (erInfo && erInfo.data.length >= 148) break;
    }
  } else {
    console.log("[2/8] delegate_arena skipped (PDA already delegated to ER)");
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
  const startSig = await sendErTx(er, startIx, payer);
  report.txs.start_round = { layer: "ER", sig: startSig, erRpc: erUrl };
  console.log(`[3/8] start_round VRF request (ER, 0-gas): ${startSig}`);

  // Wait for scoped #[vrf_callback] fulfillment inside ER
  let state;
  for (let i = 0; i < 35; i++) {
    const info = await er.getAccountInfo(arenaPda);
    if (info && info.data.length >= 148) {
      state = parseArenaState(info.data);
      if (state.phase === 2) break;
    }
    await sleep(1000);
  }
  if (!state || state.phase !== 2) {
    throw new Error(`Timed out waiting for callback_round_vrf inside ER (phase=${state?.phase})`);
  }
  const erSigsAfterRoundVrf = await er.getSignaturesForAddress(arenaPda, { limit: 5 });
  const roundCbSig = erSigsAfterRoundVrf.find((s) => s.signature !== startSig)?.signature || null;
  report.txs.callback_round_vrf = {
    layer: "ER",
    sig: roundCbSig,
    crashPointBps: state.crashPointBps,
    vrfHash: state.lastVrfHash,
  };
  console.log(
    `[4/8] callback_round_vrf fulfilled on ER: crash_point=${(state.crashPointBps / 10000).toFixed(2)}x (cbSig=${roundCbSig})`
  );

  // Step 4: Cashout inside ER (1.10x target)
  const cashoutIx = new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: payer.publicKey, isSigner: true, isWritable: true },
      { pubkey: arenaPda, isSigner: false, isWritable: true },
    ],
    data: Buffer.concat([disc("cashout_round"), u32Le(11_000)]),
  });
  const cashSig = await sendErTx(er, cashoutIx, payer);
  const afterCashInfo = await er.getAccountInfo(arenaPda);
  const afterCash = parseArenaState(afterCashInfo.data);
  report.txs.cashout_round = {
    layer: "ER",
    sig: cashSig,
    targetBps: 11_000,
    chipsAfter: afterCash.chips,
    shardsAfter: afterCash.shards,
    winStreak: afterCash.winStreak,
  };
  console.log(
    `[5/8] cashout_round 1.10x (ER, 0-gas): ${cashSig} -> chips=${afterCash.chips}, shards=${afterCash.shards}, streak=${afterCash.winStreak}`
  );

  // Step 5: Open VRF Relic Pack inside ER (burns 100 shards -> mints Common/Rare/Epic/Mythic Relic)
  if (afterCash.shards >= 100) {
    const relicsBefore = afterCash.totalRelics;
    const packIx = new TransactionInstruction({
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
      data: Buffer.concat([disc("open_relic_pack"), Buffer.from([99])]),
    });
    const packSig = await sendErTx(er, packIx, payer);
    report.txs.open_relic_pack = { layer: "ER", sig: packSig, erRpc: erUrl };
    console.log(`[6/8] open_relic_pack VRF request (ER, 0-gas): ${packSig}`);

    let afterRelic;
    for (let i = 0; i < 35; i++) {
      const info = await er.getAccountInfo(arenaPda);
      if (info && info.data.length >= 148) {
        const s = parseArenaState(info.data);
        if (s.phase === 0 && s.totalRelics > relicsBefore) {
          afterRelic = s;
          break;
        }
      }
      await sleep(1000);
    }
    if (!afterRelic) {
      throw new Error("Timed out waiting for callback_relic_vrf inside ER");
    }
    const erSigsAfterRelic = await er.getSignaturesForAddress(arenaPda, { limit: 5 });
    const relicCbSig = erSigsAfterRelic.find((s) => s.signature !== packSig)?.signature || null;
    report.txs.callback_relic_vrf = {
      layer: "ER",
      sig: relicCbSig,
      lastRelicTier: afterRelic.lastRelicTier,
      totalRelics: afterRelic.totalRelics,
      vrfHash: afterRelic.lastVrfHash,
    };
    console.log(
      `      callback_relic_vrf fulfilled on ER: tier=${afterRelic.lastRelicTier}, totalRelics=${afterRelic.totalRelics} (cbSig=${relicCbSig})`
    );
  }

  // Step 6: Checkpoint state to L1 while staying delegated (commit_arena)
  const commitIx = new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: payer.publicKey, isSigner: true, isWritable: true },
      { pubkey: arenaPda, isSigner: false, isWritable: true },
      { pubkey: MAGIC_PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: MAGIC_CONTEXT_ID, isSigner: false, isWritable: true },
    ],
    data: disc("commit_arena"),
  });
  const commitSig = await sendErTx(er, commitIx, payer);
  report.txs.commit_arena = { layer: "ER", sig: commitSig };
  console.log(`[7/8] commit_arena checkpoint (ER -> L1): ${commitSig}`);
  await sleep(2000);

  // Step 7: Commit & Undelegate back to Solana L1 via MagicIntentBundleBuilder
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
  const undelSig = await sendErTx(er, undelIx, payer);
  report.txs.undelegate_arena = { layer: "ER", sig: undelSig };
  console.log(`[8/8] undelegate_arena (ER -> L1): ${undelSig}`);

  // Wait for Solana L1 ArenaState PDA ownership to return from Delegation Program to BlitzVault
  let finalL1State = null;
  for (let i = 0; i < 30; i++) {
    await sleep(1500);
    const info = await l1.getAccountInfo(arenaPda);
    if (info && info.owner.equals(PROGRAM_ID)) {
      finalL1State = parseArenaState(info.data);
      break;
    }
  }
  if (!finalL1State) {
    throw new Error("Timed out waiting for ArenaState PDA undelegation to settle on Solana L1");
  }

  // Capture the L1 commit/undelegate settlement signatures for ArenaState PDA
  const l1PdaSigs = await l1.getSignaturesForAddress(arenaPda, { limit: 10 });
  report.l1ArenaHistory = l1PdaSigs.map((s) => ({
    sig: s.signature,
    slot: s.slot,
    err: s.err,
    explorer: explorerTx(s.signature),
  }));
  report.finalL1State = finalL1State;
  report.balanceAfterLamports = await l1.getBalance(payer.publicKey);

  console.log("\n=== FINAL VERIFIED SOLANA DEVNET L1 ARENA STATE ===");
  console.log(
    `Owner=${PROGRAM_ID.toBase58()} | chips=${finalL1State.chips} | shards=${finalL1State.shards} | rounds=${finalL1State.roundsPlayed} | relics=${finalL1State.totalRelics} (tier=${finalL1State.lastRelicTier}) | vrfHash=${finalL1State.lastVrfHash.slice(0, 16)}...`
  );
  console.log("L1 Arena PDA Settlement TXs:");
  for (const h of report.l1ArenaHistory) {
    console.log(`  slot=${h.slot} err=${h.err} ${h.explorer}`);
  }
  console.log(
    `Remaining Deployer Balance: ${(report.balanceAfterLamports / 1e9).toFixed(6)} Devnet SOL`
  );

  if (process.env.POW_JSON_OUT) {
    fs.writeFileSync(process.env.POW_JSON_OUT, JSON.stringify(report, null, 2));
    console.log(`Saved JSON proof report to ${process.env.POW_JSON_OUT}`);
  }
  console.log("\n=== FULL DEVNET + MAGICBLOCK ER PROOF-OF-WORK COMPLETE ===");
}

main().catch((err) => {
  console.error("Devnet PoW error:", err.message || err);
  if (err.logs) console.error("Program logs:", err.logs);
  process.exit(1);
});
