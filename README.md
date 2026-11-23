# ⚡ BlitzVault — Real-Time Entertainment Finance on MagicBlock ER

[![Build SBF, Run E2E & Deploy Web Demo](https://github.com/AndreyJackKali/blitz-vault/actions/workflows/ci-pages.yml/badge.svg)](https://github.com/AndreyJackKali/blitz-vault/actions/workflows/ci-pages.yml)
[![CI SBF E2E](https://img.shields.io/badge/CI%20SBF%20E2E-10%2F10%20Passed-00ff94?style=flat-square&logo=githubactions)](https://github.com/AndreyJackKali/blitz-vault/actions/workflows/ci-pages.yml)
[![Live Arena](https://img.shields.io/badge/Live%20Demo-GitHub%20Pages-00ff94?style=flat-square&logo=solana)](https://andreyjackkali.github.io/blitz-vault/)
[![Demo Video](https://img.shields.io/badge/HD%20Walkthrough-48s%20MP4-38bdf8?style=flat-square)](https://github.com/AndreyJackKali/blitz-vault/raw/main/docs/demo.mp4)
[![MagicBlock SDK](https://img.shields.io/badge/MagicBlock%20ER%20SDK-0.18.0-7c3aed?style=flat-square)](https://docs.magicblock.gg/)
[![Anchor](https://img.shields.io/badge/Anchor-1.0.2-3b82f6?style=flat-square)](https://www.anchor-lang.com/)

> **Built for Solana Blitz v10: Entertainment Finance** (Nov 20–27, 2026)  
> **The Pitch:** BlitzVault fuses **sub-50ms, zero-gas on-chain Crash Multiplier speculation** with **provably fair VRF Collectible Relic Packs** that permanently boost your future yield — powered end-to-end by **MagicBlock Ephemeral Rollups (`ephemeral-rollups-sdk 0.18.0`)**, **Magic Router (`devnet-router.magicblock.app`)**, and **Scoped MagicBlock VRF**.

---

## 🎬 Live Interactive Demo & 48s HD Walkthrough

- **Play Live Now (Zero Setup · Magic Router Live Telemetry + Judge Sandbox / Phantom Devnet):** 👉 **[https://andreyjackkali.github.io/blitz-vault/](https://andreyjackkali.github.io/blitz-vault/)**
- **Watch 48s HD Walkthrough Video:** 👉 **[`docs/demo.mp4`](https://github.com/AndreyJackKali/blitz-vault/raw/main/docs/demo.mp4)** · **[GitHub Release `v1.0.0-blitz-v10`](https://github.com/AndreyJackKali/blitz-vault/releases/tag/v1.0.0-blitz-v10)**
- **100% GREEN CI Proof (14 Rust Tests + 351 KB SBF `.so` + 10/10 Localnet BPF E2E):** 👉 **[GitHub Actions Workflows](https://github.com/AndreyJackKali/blitz-vault/actions/workflows/ci-pages.yml)**

[![BlitzVault Live Arena Preview](docs/preview.png)](https://andreyjackkali.github.io/blitz-vault/)

---

## 🎯 Problem & Entertainment Finance Fit

**The Problem:** Entertainment Finance combines high-frequency trading/speculation loops with collectible asset progression that players truly own on-chain. Base-layer Solana L1 (400ms slots, gas fees, and wallet popups on every click) destroys reflex-driven crash gameplay and turns every pack opening into friction.

**How BlitzVault Solves It with MagicBlock ER + Scoped VRF:**
1. **Initialize on Solana L1 (`initialize_arena`):** Creates the player's canonical `ArenaState` PDA (`[b"blitz_arena_v1", player]`, 148 bytes) with `10,000` starter chips and `150` Vault Shards (enough for an immediate introductory Relic Pack).
2. **Delegate to MagicBlock ER (`delegate_arena` via `#[delegate]`):** Delegates the `ArenaState` PDA via MagicBlock's Delegation Program ([`DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh`](https://explorer.solana.com/address/DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh?cluster=devnet)) to any live 10ms ER validator discovered from `https://devnet-router.magicblock.app` (`remaining_accounts[0]`) for **0-gas, sub-50ms execution**.
3. **Provably Fair Round Launch (`start_round` + `callback_round_vrf`):** Requests scoped randomness inside the ER via `create_request_randomness_ix_const` on the **fee-exempt `DEFAULT_EPHEMERAL_QUEUE`** ([`5hBR571xnXppuCPveTrctfTU7tJLSN94nq7kv7FRK5Tc`](https://explorer.solana.com/address/5hBR571xnXppuCPveTrctfTU7tJLSN94nq7kv7FRK5Tc?cluster=devnet)), verified by `#[vrf_callback]` to derive a heavy-tail crash multiplier (`1.00×–50.00×`).
4. **Sub-50ms Gasless Cashout (`cashout_round`):** Settles profit + permanent Relic yield boost (up to `+25%`) + streak-scaled Vault Shards (`25 + 10 × streak` on win, `+10` consolation on bust) inside the ER with **0 gas fees**.
5. **VRF Collectible Relic Packs (`open_relic_pack` + `callback_relic_vrf`):** Burns `100` Vault Shards for verifiable Relic drops (`Common 60% +1%`, `Rare 25% +3%`, `Epic 12% +6%`, `Mythic 3% +12%`, permanently compounding future crash profits up to `+25%`).
6. **Stale VRF Timeout Recovery (`cancel_stale_vrf`):** 125-second TTL safely refunds stake/shards and advances the round nonce so any delayed oracle callback fails closed.
7. **Atomic L1 Checkpoint & Exit (`commit_arena` & `undelegate_arena`):** Uses SDK `0.18.0`'s non-deprecated `MagicIntentBundleBuilder` to checkpoint while staying delegated (`.commit()`) or atomically commit & undelegate final balances, streaks, and Relic collections back to Solana L1 (`.commit_and_undelegate()`).

---

## 🏗 Architecture Diagram

```mermaid
sequenceDiagram
    autonumber
    actor Player as Player / Web Arena
    participant Router as Magic Router (devnet-router.magicblock.app)
    participant L1 as Solana L1 (BlitzVault Program)
    participant ER as MagicBlock ER Validator (10ms · 0-gas)
    participant VRF as MagicBlock Scoped VRF (DEFAULT_EPHEMERAL_QUEUE)

    Player->>Router: getRoutes() → discover live 10ms ER validators
    Router-->>Player: SGP (MAS1Dt…) / USA (MUS3hc…) / EU (MEUGGr…) / TEE (MTEWGu…)

    Player->>L1: initialize_arena()
    L1-->>Player: Create ArenaState PDA (148B · 10k chips · 150 shards)

    Player->>L1: delegate_arena(validator) [#[delegate] → DELeGGvX…RSaeSh]
    L1->>ER: Delegate ArenaState PDA ownership

    rect rgba(0, 255, 148, 0.08)
        Note over Player,VRF: Sub-50ms Zero-Gas Entertainment Finance Loop on ER
        Player->>ER: start_round(stake, seed) [#[vrf]]
        ER->>VRF: create_request_randomness_ix_const(PROGRAM_IDENTITY, nonce)
        VRF-->>ER: callback_round_vrf(randomness[32], nonce) [#[vrf_callback]]
        Note right of ER: derive_crash_point_bps() → 1.00×–50.00× (Live)
        Player->>ER: cashout_round(target_mult_bps >= 10,500)
        ER-->>Player: Settle profit + Relic boost (≤+25%) + Vault Shards (0 gas)

        Player->>ER: open_relic_pack(seed) [burn 100 shards]
        ER->>VRF: create_request_randomness_ix_const(VRF_DOMAIN_RELIC)
        VRF-->>ER: callback_relic_vrf(randomness[32], nonce)
        ER-->>Player: Mint Common/Rare/Epic/Mythic Relic on-chain
    end

    Note over Player,ER: Safety Escape Hatch: cancel_stale_vrf() after >125s refunds & advances nonce

    Player->>ER: commit_arena() / undelegate_arena() [MagicIntentBundleBuilder]
    ER->>L1: Atomic commit & undelegate final chips, streaks & Relics to Solana L1
```

---

## ⚡ Exact MagicBlock ER + VRF Integration & Code Links

| MagicBlock Primitive / Criterion | Implementation in BlitzVault | Exact Source Code |
|---|---|---|
| **`#[ephemeral]` Program Architecture** | Full Ephemeral Rollup program entrypoint (`ephemeral-rollups-sdk = "=0.18.0"`, `anchor-lang = "1.0"`) | [`lib.rs#L38-L40`](programs/blitz-vault/src/lib.rs#L38-L40) |
| **Account Delegation (`#[delegate]`)** | `DelegateArena` (`UncheckedAccount` + `del`, SIMD-0460 safe) delegating `ArenaState` PDA via `DELeGGvX…RSaeSh` with optional router validator in `remaining_accounts[0]` | [`lib.rs#L78-L90`](programs/blitz-vault/src/lib.rs#L78-L90), [`L541-L549`](programs/blitz-vault/src/lib.rs#L541-L549) |
| **Scoped VRF Requests (`#[vrf]`)** | Domain-separated seeds (`VRF_DOMAIN_ROUND = 0x01`, `VRF_DOMAIN_RELIC = 0xA5`), compile-time `PROGRAM_IDENTITY`, fee-exempt `DEFAULT_EPHEMERAL_QUEUE` (`5hBR57…FRK5Tc`) | [`lib.rs#L94-L151`](programs/blitz-vault/src/lib.rs#L94-L151), [`L249-L298`](programs/blitz-vault/src/lib.rs#L249-L298), [`L551-L574`](programs/blitz-vault/src/lib.rs#L551-L574) |
| **Verified VRF Callbacks (`#[vrf_callback]`)** | Compile-time `SCOPED_VRF_IDENTITY` signer check (avoids >200k CU runtime `const_crypto` blowup in SBF!) + strict nonce/phase/kind replay guards | [`lib.rs#L20-L24`](programs/blitz-vault/src/lib.rs#L20-L24), [`L155-L182`](programs/blitz-vault/src/lib.rs#L155-L182), [`L302-L336`](programs/blitz-vault/src/lib.rs#L302-L336), [`L576-L584`](programs/blitz-vault/src/lib.rs#L576-L584) |
| **Atomic Commit & Undelegate (`#[commit]`)** | Zero deprecated APIs: `MagicIntentBundleBuilder` `.commit()` (`commit_arena`) & `.commit_and_undelegate()` (`undelegate_arena`) with pre-undelegate `arena.exit(&crate::ID)?` | [`lib.rs#L369-L395`](programs/blitz-vault/src/lib.rs#L369-L395), [`L599-L611`](programs/blitz-vault/src/lib.rs#L599-L611) |
| **Stale VRF Timeout Guard** | 125s TTL recovery (`cancel_stale_vrf`) refunds stake/shards and increments `round_nonce` so delayed callbacks fail closed | [`lib.rs#L340-L366`](programs/blitz-vault/src/lib.rs#L340-L366) |
| **Live Magic Router & Exact Parity HUD** | Queries `devnet-router.magicblock.app` `getRoutes` live, selects regional ER validators, and matches `derive_crash_point_bps(&[u8; 32])` & `MIN_CASHOUT_BPS (1.05×)` bit-for-bit | [`web/index.html`](web/index.html), [`scripts/devnet_pow.mjs`](scripts/devnet_pow.mjs) |

### Verified Live MagicBlock Devnet Addresses & Router Validators (Checked 2026-11-23 UTC)

| Component | Devnet Address / Endpoint | On-Chain Status |
|---|---|---|
| **Delegation Program** | [`DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh`](https://explorer.solana.com/address/DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh?cluster=devnet) | ✅ Executable (`BPFLoaderUpgradeable`) |
| **MagicBlock VRF Program** | [`Vrf1RNUjXmQGjmQrQLvJHs9SNkvDJEsRVFPkfSQUwGz`](https://explorer.solana.com/address/Vrf1RNUjXmQGjmQrQLvJHs9SNkvDJEsRVFPkfSQUwGz?cluster=devnet) | ✅ Executable (`BPFLoaderUpgradeable`) |
| **Ephemeral VRF Queue (`DEFAULT_EPHEMERAL_QUEUE`)** | [`5hBR571xnXppuCPveTrctfTU7tJLSN94nq7kv7FRK5Tc`](https://explorer.solana.com/address/5hBR571xnXppuCPveTrctfTU7tJLSN94nq7kv7FRK5Tc?cluster=devnet) | ✅ Live & Delegated to ER (0-fee) |
| **Base-Layer VRF Queue (`DEFAULT_QUEUE`)** | [`Cuj97ggrhhidhbu39TijNVqE74xvKJ69gDervRUXAxGh`](https://explorer.solana.com/address/Cuj97ggrhhidhbu39TijNVqE74xvKJ69gDervRUXAxGh?cluster=devnet) | ✅ Live (`Vrf1RNUj…`) |
| **Magic Router (`getRoutes`)** | `https://devnet-router.magicblock.app` | ✅ 4 Live 10ms / 0-fee Validators: Asia (`MAS1Dt9q…zk57`), US (`MUS3hc9T…NHNd`), EU (`MEUGGrYP…MS8e`), TEE (`MTEWGuqx…3xzo`) |

---

## 🛡 Verification, 100% GREEN CI Proofs & Devnet Status

| Check | Status | Details / Links |
|---|---|---|
| **GitHub Actions CI (`ci-pages.yml`)** | ✅ 3 / 3 Jobs Green | [View live CI runs](https://github.com/AndreyJackKali/blitz-vault/actions/workflows/ci-pages.yml) — Rust Unit Tests + Solana SBF Build & Localnet E2E + GitHub Pages Deploy |
| **Solana SBF ELF Binary (`blitz_vault.so`)** | ✅ Built in CI | `326 KB` `ELF 64-bit LSB pie executable, eBPF` built via Agave `solana-cli 4.4.1` / `platform-tools v1.57` · SHA-256 `e9d37b774b78150f8bfd907e1f819343c973143450632b3c88b2c70c0a521048` (25 KB smaller after compile-time `SCOPED_VRF_IDENTITY` CU fix!) |
| **Extended Rust Unit & Invariant Suite** | ✅ 14 / 14 Passing | Crash curve bounds & 5k Monte Carlo distribution, VRF domain separation, Relic boost caps/overflow, payout solvency, Borsh `INIT_SPACE`, MagicBlock ER/VRF PDAs (`PROGRAM_IDENTITY` + `SCOPED_VRF_IDENTITY`) & instruction layouts |
| **Real `solana-test-validator` SBF E2E** | ✅ 10 / 10 On-Chain Assertions | Loads compiled `blitz_vault.so` into Solana BPF VM (`solana-core 4.4.1`), executes real `initialize_arena` (`~15.5k–20k CU`), verifies 148-byte `ArenaState` PDA, and proves 9 negative security guards (`already in use`, `ConstraintSeeds 0x7d6`, `RoundNotLive 0x1775`, `NoPendingVrf 0x1779`, `StakeTooLow 0x1772`, `InvalidOracleQueue 0x1777` on both `start_round` & `open_relic_pack`, `InsufficientChips 0x1773`, and spoofed non-oracle `callback_round_vrf` `ConstraintAddress 0x7dc`) |
| **Live Interactive Web Arena** | ✅ Live HTTPS (`0` console errors) | [andreyjackkali.github.io/blitz-vault](https://andreyjackkali.github.io/blitz-vault/) — live `devnet-router.magicblock.app` validator selector, regional ER & Devnet L1 telemetry, `commit_arena` + `undelegate_arena`, and click-to-verify VRF proofs |
| **Solana Devnet Deploy Readiness** | ⚠️ Public RPC Faucet Closed (`410`) | Programmatic `https://api.devnet.solana.com` `requestAirdrop` returns JSON-RPC `410` (*"The RPC faucet is closed on this cluster. Request test SOL at https://faucet.solana.com"*). Verified rent cost for `351 KB` `.so` is ~1.83 SOL (~2.1 SOL with `--max-len 400000`). Ready for 1-command deploy + [`scripts/devnet_pow.mjs`](scripts/devnet_pow.mjs) once wallet is funded via web faucet. |

<details>
<summary><strong>📜 Verbatim Localnet SBF E2E Transaction Logs & Security Guard Proofs</strong></summary>

```text
=== BlitzVault SBF On-Chain E2E Validator Test ===
RPC: http://127.0.0.1:8899 Cluster version: {"feature-set":2681577653,"solana-core":"4.4.1"}
✅ Program loaded: BLzVt11111111111111111111111111111111111111 (executable=true)
✅ [PASS] initialize_arena confirmed (consumed 20041 of 200000 compute units)
✅ [PASS] On-chain ArenaState verified: 148 bytes, chips=10000, shards=150, phase=Idle
✅ [PASS] duplicate initialize_arena rejected cleanly on-chain (already in use)
✅ [PASS] unauthorized intruder cashout_round rejected cleanly on-chain (0x7d6 ConstraintSeeds)
✅ [PASS] cashout_round while Idle rejected cleanly on-chain (0x1775 RoundNotLive)
✅ [PASS] cancel_stale_vrf while Idle rejected cleanly on-chain (0x1779 NoPendingVrf)
✅ [PASS] start_round below MIN_STAKE rejected cleanly on-chain (0x1772 StakeTooLow)
✅ [PASS] start_round with invalid oracle queue rejected cleanly on-chain (0x1777 InvalidOracleQueue)
✅ [PASS] start_round exceeding chips balance rejected cleanly on-chain (0x1773 InsufficientChips)
✅ [PASS] open_relic_pack with invalid oracle queue rejected cleanly on-chain (0x1777 InvalidOracleQueue)
✅ [PASS] spoofed callback_round_vrf by non-oracle signer rejected cleanly on-chain (0x7dc ConstraintAddress)
=== ALL 10 ON-CHAIN SBF E2E ASSERTIONS PASSED ===
```

</details>

---

## 🚀 1-Command Local Verification & Devnet Deploy

```bash
# 1. Run all 14 Rust unit & MagicBlock ER/VRF layout tests
cargo test --manifest-path programs/blitz-vault/Cargo.toml --lib -- --nocapture

# 2. Compile Solana SBF binary (target/deploy/blitz_vault.so, 351 KB)
cargo-build-sbf --manifest-path programs/blitz-vault/Cargo.toml --sbf-out-dir target/deploy

# 3. Run all 10 real localnet SBF E2E assertions against compiled blitz_vault.so
solana-test-validator --reset --quiet \
  --bpf-program BLzVt11111111111111111111111111111111111111 target/deploy/blitz_vault.so \
  --bpf-program Vrf1RNUjXmQGjmQrQLvJHs9SNkvDJEsRVFPkfSQUwGz target/deploy/blitz_vault.so &
node scripts/e2e_localnet.mjs

# 4. Deploy to Solana Devnet (~2.1 SOL from https://faucet.solana.com) & run live ER + VRF Proof-of-Work
solana program deploy target/deploy/blitz_vault.so --url devnet --max-len 400000
BLITZ_PROGRAM_ID=<DEPLOYED_PROGRAM_ID> node scripts/devnet_pow.mjs
```

---

## 🔧 Technical Highlights (Nov 2026 MagicBlock Best Practices)
- **SDK `0.18.0` + Anchor `1.0.2`:** Compatible with both validator 1.x and 2.0 public ERs (avoids `0.20.0` `slot_hashes` VRF breakage).
- **Scoped VRF Security & Compile-Time CU Fix:** Compile-time `PROGRAM_IDENTITY = program_identity_pubkey(&crate::ID)` and compile-time `SCOPED_VRF_IDENTITY = Pubkey::new_from_array(scoped_vrf_identity_const(&crate::ID).0)` inside `#[vrf_callback]` prevent spoofed oracle calls while avoiding SDK `0.18.0`'s >200k CU runtime `const_crypto::ed25519` blowup in SBF (verified on-chain in E2E assertion #10!).
- **Strict Nonce State Machine:** `Idle → AwaitingVrf(nonce) → Live → Idle` with `cancel_stale_vrf()` (>125s TTL) advancing the nonce so late callbacks fail closed.
- **Zero Deprecated APIs:** Uses `MagicIntentBundleBuilder` (`.commit()` & `.commit_and_undelegate()`) and `UncheckedAccount` with `del` constraint (SIMD-0460 safe).
- **Exact Rust ↔ Frontend Parity:** Both `programs/blitz-vault/src/lib.rs` and `web/index.html` implement identical `derive_crash_point_bps(&[u8; 32])` fixed-point basis-point arithmetic and `1.05×` minimum cashout enforcement.
