import crypto from "node:crypto";
import assert from "node:assert/strict";
import {
  Connection,
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
  TransactionInstruction,
  sendAndConfirmTransaction,
} from "@solana/web3.js";

const RPC_URL = process.env.SOLANA_RPC_URL || "http://127.0.0.1:8899";
const PROGRAM_ID = new PublicKey("BLzVt11111111111111111111111111111111111111");
const DELEGATION_PROGRAM_ID = new PublicKey("DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh");
const DEFAULT_EPHEMERAL_QUEUE = new PublicKey("5hBR571xnXppuCPveTrctfTU7tJLSN94nq7kv7FRK5Tc");
const ARENA_SEED = Buffer.from("blitz_arena_v1");

function anchorDisc(name) {
  return crypto.createHash("sha256").update(`global:${name}`).digest().subarray(0, 8);
}

function u64Le(n) {
  const b = Buffer.alloc(8);
  b.writeBigUInt64LE(BigInt(n), 0);
  return b;
}

function u32Le(n) {
  const b = Buffer.alloc(4);
  b.writeUInt32LE(n, 0);
  return b;
}

function parseArenaState(data) {
  assert.ok(data.length >= 145, `Unexpected ArenaState size: ${data.length}`);
  let off = 8; // skip 8-byte Anchor account discriminator
  const player = new PublicKey(data.subarray(off, off + 32)); off += 32;
  const chips = data.readBigUInt64LE(off); off += 8;
  const shards = data.readBigUInt64LE(off); off += 8;
  const roundNonce = data.readBigUInt64LE(off); off += 8;
  const phase = data.readUInt8(off); off += 1;
  const pendingVrfKind = data.readUInt8(off); off += 1;
  const activeStake = data.readBigUInt64LE(off); off += 8;
  const crashPointBps = data.readUInt32LE(off); off += 4;
  const lastCashoutBps = data.readUInt32LE(off); off += 4;
  const lastVrfRequestTs = data.readBigInt64LE(off); off += 8;
  const winStreak = data.readUInt16LE(off); off += 2;
  const bestStreak = data.readUInt16LE(off); off += 2;
  const roundsPlayed = data.readUInt32LE(off); off += 4;
  const totalPnl = data.readBigInt64LE(off); off += 8;
  const relicsCommon = data.readUInt16LE(off); off += 2;
  const relicsRare = data.readUInt16LE(off); off += 2;
  const relicsEpic = data.readUInt16LE(off); off += 2;
  const relicsMythic = data.readUInt16LE(off); off += 2;
  const lastRelicTier = data.readUInt8(off); off += 1;
  const lastVrfHash = data.subarray(off, off + 32).toString("hex"); off += 32;
  const bump = data.readUInt8(off); off += 1;
  return {
    byteLength: data.length,
    decodedBytes: off,
    player: player.toBase58(),
    chips: Number(chips),
    shards: Number(shards),
    roundNonce: Number(roundNonce),
    phase,
    pendingVrfKind,
    activeStake: Number(activeStake),
    crashPointBps,
    lastCashoutBps,
    lastVrfRequestTs: Number(lastVrfRequestTs),
    winStreak,
    bestStreak,
    roundsPlayed,
    totalPnl: Number(totalPnl),
    relicsCommon,
    relicsRare,
    relicsEpic,
    relicsMythic,
    lastRelicTier,
    lastVrfHash,
    bump,
  };
}

async function expectError(connection, payer, ix, expectedSubstrings, label) {
  const { blockhash } = await connection.getLatestBlockhash("confirmed");
  const tx = new Transaction({ recentBlockhash: blockhash, feePayer: payer.publicKey }).add(ix);
  tx.sign(payer);
  const res = await fetch(RPC_URL, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "simulateTransaction",
      params: [
        tx.serialize().toString("base64"),
        { encoding: "base64", commitment: "confirmed", sigVerify: true },
      ],
    }),
  });
  const json = await res.json();
  assert.ok(!json.error, `${label}: RPC error ${JSON.stringify(json.error)}`);
  const val = json.result?.value;
  assert.ok(val && val.err, `${label}: expected on-chain SBF error, but simulation succeeded`);
  const logs = val.logs || [];
  const combined = JSON.stringify(val.err) + " " + logs.join("\n");
  const matched = expectedSubstrings.some((s) => combined.includes(s));
  assert.ok(
    matched,
    `${label}: expected one of ${JSON.stringify(expectedSubstrings)}, got err=${JSON.stringify(val.err)} logs=${JSON.stringify(logs)}`
  );
  console.log(`✅ [PASS] ${label} rejected cleanly on-chain (${expectedSubstrings[0]})`);
  const errLine = logs.find((l) => l.includes("Error") || l.includes("already in use") || l.includes("failed"));
  if (errLine) console.log("   BPF Log:", errLine);
}

async function main() {
  const connection = new Connection(RPC_URL, "confirmed");
  const version = await connection.getVersion();
  console.log("=== BlitzVault SBF On-Chain E2E Validator Test ===");
  console.log("RPC:", RPC_URL, "Cluster version:", JSON.stringify(version));

  const progInfo = await connection.getAccountInfo(PROGRAM_ID);
  assert.ok(progInfo && progInfo.executable, "BlitzVault SBF program must be loaded and executable");
  console.log(`✅ Program loaded: ${PROGRAM_ID.toBase58()} (executable=${progInfo.executable})`);

  const player = Keypair.generate();
  const intruder = Keypair.generate();
  for (const kp of [player, intruder]) {
    const sig = await connection.requestAirdrop(kp.publicKey, 5_000_000_000);
    const bh = await connection.getLatestBlockhash("confirmed");
    await connection.confirmTransaction({ signature: sig, ...bh }, "confirmed");
  }
  console.log(`✅ Funded test player  : ${player.publicKey.toBase58()} (5 SOL localnet)`);
  console.log(`✅ Funded test intruder: ${intruder.publicKey.toBase58()} (5 SOL localnet)`);

  const [arenaPda, canonicalBump] = PublicKey.findProgramAddressSync(
    [ARENA_SEED, player.publicKey.toBuffer()],
    PROGRAM_ID
  );
  const [programIdentity] = PublicKey.findProgramAddressSync([Buffer.from("identity")], PROGRAM_ID);
  const [bufferPda] = PublicKey.findProgramAddressSync([Buffer.from("buffer"), arenaPda.toBuffer()], PROGRAM_ID);
  const [delegationRecord] = PublicKey.findProgramAddressSync(
    [Buffer.from("delegation"), arenaPda.toBuffer()],
    DELEGATION_PROGRAM_ID
  );
  const [delegationMetadata] = PublicKey.findProgramAddressSync(
    [Buffer.from("delegation-metadata"), arenaPda.toBuffer()],
    DELEGATION_PROGRAM_ID
  );

  console.log("Derived PDAs:");
  console.log("  ArenaState PDA      :", arenaPda.toBase58(), `(bump=${canonicalBump})`);
  console.log("  VRF ProgramIdentity :", programIdentity.toBase58());
  console.log("  ER Buffer PDA       :", bufferPda.toBase58());
  console.log("  ER DelegationRecord :", delegationRecord.toBase58());
  console.log("  ER DelegationMeta   :", delegationMetadata.toBase58());

  // 1. Real on-chain initialize_arena SBF execution
  const initIx = new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: arenaPda, isSigner: false, isWritable: true },
      { pubkey: player.publicKey, isSigner: true, isWritable: true },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    ],
    data: anchorDisc("initialize_arena"),
  });
  const initSig = await sendAndConfirmTransaction(connection, new Transaction().add(initIx), [player], {
    commitment: "confirmed",
  });
  const initTx = await connection.getTransaction(initSig, {
    commitment: "confirmed",
    maxSupportedTransactionVersion: 0,
  });
  console.log(`✅ [PASS] initialize_arena confirmed: ${initSig}`);
  console.log("   BPF Program Logs:");
  for (const line of initTx?.meta?.logMessages || []) {
    console.log("    ", line);
  }

  // Decode & verify on-chain ArenaState account bytes
  const arenaAcct = await connection.getAccountInfo(arenaPda);
  assert.ok(arenaAcct, "ArenaState PDA must exist on-chain after initialize_arena");
  assert.equal(arenaAcct.owner.toBase58(), PROGRAM_ID.toBase58());
  const parsed = parseArenaState(arenaAcct.data);
  assert.equal(parsed.player, player.publicKey.toBase58());
  assert.equal(parsed.chips, 10_000);
  assert.equal(parsed.shards, 150);
  assert.equal(parsed.phase, 0);
  assert.equal(parsed.bump, canonicalBump);
  assert.equal(parsed.byteLength, parsed.decodedBytes);
  console.log(
    `✅ [PASS] On-chain ArenaState verified: ${parsed.byteLength} bytes, chips=${parsed.chips}, shards=${parsed.shards}, phase=Idle`
  );

  // 2. Duplicate initialize_arena must fail (account already in use)
  await expectError(
    connection,
    player,
    initIx,
    ["already in use", "0x0"],
    "duplicate initialize_arena"
  );

  // 3. Unauthorized signer calling cashout_round on player's PDA must fail (ConstraintSeeds 0x7d6 or Unauthorized 0x1770)
  const unauthIx = new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: intruder.publicKey, isSigner: true, isWritable: true },
      { pubkey: arenaPda, isSigner: false, isWritable: true },
    ],
    data: Buffer.concat([anchorDisc("cashout_round"), u32Le(20_000)]),
  });
  await expectError(
    connection,
    intruder,
    unauthIx,
    ["0x7d6", "2006", "ConstraintSeeds", "0x1770", "6000"],
    "unauthorized intruder cashout_round"
  );

  // 4. Verify cashout_round fails closed when phase == Idle (BlitzError::RoundNotLive = 6005 = 0x1775)
  const cashoutIdleIx = new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: player.publicKey, isSigner: true, isWritable: true },
      { pubkey: arenaPda, isSigner: false, isWritable: true },
    ],
    data: Buffer.concat([anchorDisc("cashout_round"), u32Le(20_000)]),
  });
  await expectError(connection, player, cashoutIdleIx, ["0x1775", "6005", "RoundNotLive"], "cashout_round while Idle");

  // 5. Verify cancel_stale_vrf fails closed when no VRF pending (BlitzError::NoPendingVrf = 6009 = 0x1779)
  const cancelIdleIx = new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: player.publicKey, isSigner: true, isWritable: true },
      { pubkey: arenaPda, isSigner: false, isWritable: true },
    ],
    data: anchorDisc("cancel_stale_vrf"),
  });
  await expectError(connection, player, cancelIdleIx, ["0x1779", "6009", "NoPendingVrf"], "cancel_stale_vrf while Idle");

  // 6. Verify start_round enforces MIN_STAKE on-chain (BlitzError::StakeTooLow = 6002 = 0x1772)
  const vrfProgramId = new PublicKey("Vrf1RNUjXmQGjmQrQLvJHs9SNkvDJEsRVFPkfSQUwGz");
  const slotHashesSysvar = new PublicKey("SysvarS1otHashes111111111111111111111111111");
  const lowStakeIx = new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: player.publicKey, isSigner: true, isWritable: true },
      { pubkey: arenaPda, isSigner: false, isWritable: true },
      { pubkey: DEFAULT_EPHEMERAL_QUEUE, isSigner: false, isWritable: true },
      { pubkey: programIdentity, isSigner: false, isWritable: false },
      { pubkey: vrfProgramId, isSigner: false, isWritable: false },
      { pubkey: slotHashesSysvar, isSigner: false, isWritable: false },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    ],
    data: Buffer.concat([anchorDisc("start_round"), u64Le(50), Buffer.from([7])]),
  });
  await expectError(connection, player, lowStakeIx, ["0x1772", "6002", "StakeTooLow"], "start_round below MIN_STAKE");

  // 7. Verify start_round rejects unauthorized oracle queue (BlitzError::InvalidOracleQueue = 6007 = 0x1777)
  const fakeQueue = Keypair.generate().publicKey;
  const badQueueIx = new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: player.publicKey, isSigner: true, isWritable: true },
      { pubkey: arenaPda, isSigner: false, isWritable: true },
      { pubkey: fakeQueue, isSigner: false, isWritable: true },
      { pubkey: programIdentity, isSigner: false, isWritable: false },
      { pubkey: vrfProgramId, isSigner: false, isWritable: false },
      { pubkey: slotHashesSysvar, isSigner: false, isWritable: false },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    ],
    data: Buffer.concat([anchorDisc("start_round"), u64Le(500), Buffer.from([7])]),
  });
  await expectError(
    connection,
    player,
    badQueueIx,
    ["0x1777", "6007", "InvalidOracleQueue"],
    "start_round with invalid oracle queue"
  );

  // 8. Verify start_round rejects stake exceeding chips balance (BlitzError::InsufficientChips = 6003 = 0x1773)
  const overStakeIx = new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: player.publicKey, isSigner: true, isWritable: true },
      { pubkey: arenaPda, isSigner: false, isWritable: true },
      { pubkey: DEFAULT_EPHEMERAL_QUEUE, isSigner: false, isWritable: true },
      { pubkey: programIdentity, isSigner: false, isWritable: false },
      { pubkey: vrfProgramId, isSigner: false, isWritable: false },
      { pubkey: slotHashesSysvar, isSigner: false, isWritable: false },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    ],
    data: Buffer.concat([anchorDisc("start_round"), u64Le(25_000), Buffer.from([7])]),
  });
  await expectError(
    connection,
    player,
    overStakeIx,
    ["0x1773", "6003", "InsufficientChips"],
    "start_round exceeding chips balance"
  );

  // 9. Verify open_relic_pack rejects unauthorized oracle queue (BlitzError::InvalidOracleQueue = 6007 = 0x1777)
  const badRelicQueueIx = new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: player.publicKey, isSigner: true, isWritable: true },
      { pubkey: arenaPda, isSigner: false, isWritable: true },
      { pubkey: fakeQueue, isSigner: false, isWritable: true },
      { pubkey: programIdentity, isSigner: false, isWritable: false },
      { pubkey: vrfProgramId, isSigner: false, isWritable: false },
      { pubkey: slotHashesSysvar, isSigner: false, isWritable: false },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    ],
    data: Buffer.concat([anchorDisc("open_relic_pack"), Buffer.from([9])]),
  });
  await expectError(
    connection,
    player,
    badRelicQueueIx,
    ["0x1777", "6007", "InvalidOracleQueue"],
    "open_relic_pack with invalid oracle queue"
  );

  // 10. Verify #[vrf_callback] rejects spoofed non-oracle callback_round_vrf signer on-chain
  const fakeRandomness = Buffer.alloc(32, 0x42);
  const spoofCallbackIx = new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: intruder.publicKey, isSigner: true, isWritable: false },
      { pubkey: arenaPda, isSigner: false, isWritable: true },
    ],
    data: Buffer.concat([anchorDisc("callback_round_vrf"), fakeRandomness, u64Le(1)]),
  });
  await expectError(
    connection,
    intruder,
    spoofCallbackIx,
    ["0x7dc", "2012", "ConstraintAddress", "0xbc2", "3010", "AccountNotSigner", "0x1778", "6008"],
    "spoofed callback_round_vrf by non-oracle signer"
  );

  console.log("=== ALL 10 ON-CHAIN SBF E2E ASSERTIONS PASSED ===");
}

main().catch((err) => {
  console.error("E2E test failed:", err);
  process.exit(1);
});
