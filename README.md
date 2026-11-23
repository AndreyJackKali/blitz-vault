# ⚡ BlitzVault — Real-Time Entertainment Finance on MagicBlock ER

> Built for **Solana Blitz v10: Entertainment Finance** (Nov 20–27, 2026)  
> Zero-fee, sub-second on-chain Crash Multiplier Arena + Provably Fair VRF Collectible Relic Packs powered by **MagicBlock Ephemeral Rollups (`ephemeral-rollups-sdk 0.18.0`)** & **Scoped MagicBlock VRF**.

## Why MagicBlock Ephemeral Rollups?

Entertainment Finance combines **real-time speculation** and **collectible game progression** on assets players truly own. Standard L1 transactions (400ms slots + gas fees + wallet popups) break sub-second gameplay.

**BlitzVault** solves this with a 100% on-chain MagicBlock lifecycle:
1. **Delegate (`#[delegate]`):** Player's `ArenaState` PDA is delegated from Solana Devnet L1 to MagicBlock's Ephemeral Rollup (`devnet-as.magicblock.app`).
2. **Provably Fair Round Launch (`#[vrf]` + `#[vrf_callback]`):** Requests verifiable randomness inside the ER via `create_request_randomness_ix_const` on the **fee-exempt `DEFAULT_EPHEMERAL_QUEUE`** (`5hBR571xnXppuCPveTrctfTU7tJLSN94nq7kv7FRK5Tc`), deriving a heavy-tail crash multiplier (`1.00×–50.00×`).
3. **Sub-Second Gasless Cashout:** Players cash out live on-chain inside the ER at **0 gas fees**, earning chips + **Vault Shards** scaled by win streaks.
4. **VRF Collectible Relic Packs:** Burn 100 Vault Shards to open VRF-verified Relic Packs (Common `+1%`, Rare `+3%`, Epic `+6%`, Mythic `+12%`, up to `+25%` permanent on-chain yield boost).
5. **Atomic L1 Settlement (`MagicIntentBundleBuilder`):** `commit_arena` & `undelegate_arena` settle final balances, streaks, and Relic collections back onto Solana L1 using SDK `0.18.0`'s non-deprecated `MagicIntentBundleBuilder`.

## Architecture

```
Solana Devnet L1                          MagicBlock Ephemeral Rollup (devnet-as)
────────────────                          ───────────────────────────────────────
1. initialize_arena() ──► ArenaState PDA
2. delegate_arena()   ──────────────────► Cloned PDA (0-gas, sub-50ms execution)
                                            │
                                            ├─► start_round() ──► MagicBlock VRF (DEFAULT_EPHEMERAL_QUEUE)
                                            ├─► callback_round_vrf() [#[vrf_callback] scoped identity]
                                            ├─► cashout_round() (instant ER settlement + Relic boost)
                                            ├─► open_relic_pack() ──► callback_relic_vrf() (Common..Mythic)
                                            ├─► cancel_stale_vrf() (125s timeout refund & nonce advance)
                                            │
3. Final L1 State     ◄───────────────────┴─ undelegate_arena() [MagicIntentBundleBuilder]
```

## Technical Highlights (Nov 2026 MagicBlock Best Practices)
- **SDK `0.18.0` + Anchor `1.0.2`:** Compatible with both validator 1.x and 2.0 public ERs (avoids `0.20.0` `slot_hashes` VRF breakage).
- **Scoped VRF Security:** Compile-time `PROGRAM_IDENTITY = program_identity_pubkey(&crate::ID)` + `#[vrf_callback]` prevents spoofed oracle calls.
- **Strict Nonce State Machine:** `Idle → AwaitingVrf(nonce) → Live → Idle` with `cancel_stale_vrf()` (>125s TTL) advancing the nonce so late callbacks fail closed.
- **Zero Deprecated APIs:** Uses `MagicIntentBundleBuilder` and `UncheckedAccount`/`AccountInfo` with `del` constraint (SIMD-0460 safe).

## Quick Build & Devnet Deploy

```bash
# Single-threaded safe build (2 vCPU friendly)
CARGO_BUILD_JOBS=1 anchor build

# Sync deployed program ID & deploy to Solana Devnet
anchor keys sync
solana airdrop 2 --url devnet
anchor deploy --provider.cluster devnet
```

## Live Demo
Open `web/index.html` (or visit GitHub Pages deployment) for live MagicBlock Devnet ER + Solana Devnet RPC latency probes and instant interactive gameplay.
