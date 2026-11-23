use anchor_lang::prelude::*;
use ephemeral_rollups_sdk::{
    anchor::{commit, delegate, ephemeral, vrf, vrf_callback},
    cpi::DelegateConfig,
    ephem::MagicIntentBundleBuilder,
    vrf::{
        self,
        consts::program_identity_pubkey,
        instructions::{create_request_randomness_ix_const, RequestRandomnessParams},
        types::SerializableAccountMeta,
    },
};

// Replaced with real deployed Devnet program keypair during `anchor keys sync`
declare_id!("BLzVt11111111111111111111111111111111111111");

/// Scoped VRF requester identity PDA derived at compile time (MagicBlock SDK 0.18+).
pub const PROGRAM_IDENTITY: Pubkey = program_identity_pubkey(&crate::ID);

/// Compile-time scoped VRF callback signer PDA (`PDA([b"identity", crate::ID], VRF_PROGRAM_ID)`).
/// Evaluated strictly at compile time so `CallbackArenaVrf` checks a 32-byte constant (~2k CU)
/// instead of running pure-Rust `const_crypto::ed25519` at runtime (>200k CU limit in SBF).
pub const SCOPED_VRF_IDENTITY: Pubkey =
    Pubkey::new_from_array(vrf::consts::scoped_vrf_identity_const(&crate::ID).0);

pub const ARENA_SEED: &[u8] = b"blitz_arena_v1";
pub const STARTING_CHIPS: u64 = 10_000;
pub const STARTING_SHARDS: u64 = 150;
pub const MIN_STAKE: u64 = 100;
pub const MIN_CASHOUT_BPS: u32 = 10_500;
pub const MAX_CRASH_BPS: u32 = 500_000;
pub const PACK_SHARD_COST: u64 = 100;
pub const VRF_TIMEOUT_SECONDS: i64 = 125;
pub const MAX_RELIC_BOOST_BPS: u16 = 2_500; // +25.00% max yield boost
pub const VRF_DOMAIN_ROUND: u8 = 0x01;
pub const VRF_DOMAIN_RELIC: u8 = 0xA5;

#[ephemeral]
#[program]
pub mod blitz_vault {
    use super::*;

    /// Initialize a player's ArenaState PDA on Solana Base Layer with starter chips.
    pub fn initialize_arena(ctx: Context<InitializeArena>) -> Result<()> {
        let arena = &mut ctx.accounts.arena;
        arena.player = ctx.accounts.player.key();
        arena.chips = STARTING_CHIPS;
        arena.shards = STARTING_SHARDS; // Enough for 1 introductory Relic Pack opening
        arena.round_nonce = 0;
        arena.phase = ArenaPhase::Idle as u8;
        arena.active_stake = 0;
        arena.crash_point_bps = 0;
        arena.last_cashout_bps = 0;
        arena.last_vrf_request_ts = 0;
        arena.pending_vrf_kind = VrfKind::None as u8;
        arena.win_streak = 0;
        arena.best_streak = 0;
        arena.rounds_played = 0;
        arena.total_pnl = 0;
        arena.relics_common = 0;
        arena.relics_rare = 0;
        arena.relics_epic = 0;
        arena.relics_mythic = 0;
        arena.last_relic_tier = 0;
        arena.last_vrf_hash = [0u8; 32];
        arena.bump = ctx.bumps.arena;

        msg!(
            "BlitzVault arena initialized for {} with {} chips",
            arena.player,
            arena.chips
        );
        Ok(())
    }

    /// Delegate the player's ArenaState PDA from Solana L1 to a MagicBlock Ephemeral Rollup.
    /// Pass optional ER validator pubkey as remaining_accounts[0].
    pub fn delegate_arena(ctx: Context<DelegateArena>) -> Result<()> {
        let player_key = ctx.accounts.payer.key();
        ctx.accounts.delegate_arena(
            &ctx.accounts.payer,
            &[ARENA_SEED, player_key.as_ref()],
            DelegateConfig {
                validator: ctx.remaining_accounts.first().map(|acc| acc.key()),
                ..Default::default()
            },
        )?;
        msg!("ArenaState delegated to MagicBlock ER for {}", player_key);
        Ok(())
    }

    /// Start a new crash multiplier round inside the Ephemeral Rollup by requesting
    /// provably fair randomness from MagicBlock's fee-exempt DEFAULT_EPHEMERAL_QUEUE.
    pub fn start_round(
        ctx: Context<RequestArenaVrf>,
        stake: u64,
        client_seed: u8,
    ) -> Result<()> {
        let arena = &mut ctx.accounts.arena;
        require!(
            arena.phase == ArenaPhase::Idle as u8,
            BlitzError::RoundAlreadyActive
        );
        require!(stake >= MIN_STAKE, BlitzError::StakeTooLow);
        require!(stake <= arena.chips, BlitzError::InsufficientChips);

        let now = Clock::get()?.unix_timestamp;
        let next_nonce = arena
            .round_nonce
            .checked_add(1)
            .ok_or(BlitzError::MathOverflow)?;

        arena.chips = arena
            .chips
            .checked_sub(stake)
            .ok_or(BlitzError::MathOverflow)?;
        arena.active_stake = stake;
        arena.round_nonce = next_nonce;
        arena.phase = ArenaPhase::AwaitingVrf as u8;
        arena.pending_vrf_kind = VrfKind::CrashRound as u8;
        arena.crash_point_bps = 0;
        arena.last_vrf_request_ts = now;

        let caller_seed = build_vrf_caller_seed(client_seed, next_nonce, now, VRF_DOMAIN_ROUND);

        let params = RequestRandomnessParams {
            payer: ctx.accounts.payer.key(),
            oracle_queue: ctx.accounts.oracle_queue.key(),
            callback_program_id: crate::ID,
            callback_discriminator: instruction::CallbackRoundVrf::DISCRIMINATOR.to_vec(),
            caller_seed,
            accounts_metas: Some(vec![SerializableAccountMeta {
                pubkey: arena.key(),
                is_signer: false,
                is_writable: true,
            }]),
            callback_args: Some(next_nonce.to_le_bytes().to_vec()),
            ..Default::default()
        };

        let ix = create_request_randomness_ix_const(params, PROGRAM_IDENTITY);
        ctx.accounts
            .invoke_signed_vrf(&ctx.accounts.payer.to_account_info(), &ix)?;

        msg!(
            "VRF requested for round #{} (stake={} chips)",
            next_nonce,
            stake
        );
        Ok(())
    }

    /// Scoped VRF callback invoked by the MagicBlock VRF oracle.
    /// Computes a provably fair crash point in basis points (10_000 = 1.00x, max 500_000 = 50.00x).
    pub fn callback_round_vrf(
        ctx: Context<CallbackArenaVrf>,
        randomness: [u8; 32],
        expected_nonce: u64,
    ) -> Result<()> {
        let arena = &mut ctx.accounts.arena;

        // Guard against stale or replayed VRF callbacks
        require!(
            arena.phase == ArenaPhase::AwaitingVrf as u8
                && arena.pending_vrf_kind == VrfKind::CrashRound as u8
                && arena.round_nonce == expected_nonce,
            BlitzError::StaleVrfCallback
        );

        let crash_bps = derive_crash_point_bps(&randomness);
        arena.crash_point_bps = crash_bps;
        arena.last_vrf_hash = randomness;
        arena.phase = ArenaPhase::Live as u8;
        arena.pending_vrf_kind = VrfKind::None as u8;

        msg!(
            "Round #{} VRF fulfilled: crash_point={} bps",
            expected_nonce,
            crash_bps
        );
        Ok(())
    }

    /// Sub-second gasless ER settlement: player locks in target cashout multiplier (bps).
    /// If target_mult_bps <= crash_point_bps -> WIN (payout + Relic yield boost + Vault Shards).
    /// Otherwise -> BUST (streak resets).
    pub fn cashout_round(ctx: Context<PlayArena>, target_mult_bps: u32) -> Result<()> {
        let arena = &mut ctx.accounts.arena;
        require!(
            arena.phase == ArenaPhase::Live as u8,
            BlitzError::RoundNotLive
        );
        require!(
            target_mult_bps >= MIN_CASHOUT_BPS,
            BlitzError::InvalidMultiplier
        );

        let stake = arena.active_stake;
        arena.rounds_played = arena.rounds_played.saturating_add(1);
        arena.last_cashout_bps = target_mult_bps;

        if target_mult_bps <= arena.crash_point_bps {
            let (_base_payout, bonus_profit, total_payout) =
                compute_payout(stake, target_mult_bps, arena.relic_boost_bps())?;
            let net_profit = total_payout.saturating_sub(stake);

            arena.chips = arena
                .chips
                .checked_add(total_payout)
                .ok_or(BlitzError::MathOverflow)?;
            arena.total_pnl = arena.total_pnl.saturating_add(net_profit as i64);
            arena.win_streak = arena.win_streak.saturating_add(1);
            if arena.win_streak > arena.best_streak {
                arena.best_streak = arena.win_streak;
            }

            // Award Vault Shards scaled by streak (25 base + 10 per streak)
            let shard_reward = 25u64.saturating_add((arena.win_streak as u64).saturating_mul(10));
            arena.shards = arena.shards.saturating_add(shard_reward);

            msg!(
                "WIN! cashed out @ {} bps (crash @ {}), payout={} (+{} relic bonus), +{} shards",
                target_mult_bps,
                arena.crash_point_bps,
                total_payout,
                bonus_profit,
                shard_reward
            );
        } else {
            arena.total_pnl = arena.total_pnl.saturating_sub(stake as i64);
            arena.win_streak = 0;
            // Consolation shards so players always progress toward Relic Packs
            arena.shards = arena.shards.saturating_add(10);

            msg!(
                "BUST! target {} bps > crash {} bps (lost {} chips)",
                target_mult_bps,
                arena.crash_point_bps,
                stake
            );
        }

        arena.active_stake = 0;
        arena.phase = ArenaPhase::Idle as u8;
        Ok(())
    }

    /// Burn 100 Vault Shards inside the ER to open a provably fair VRF Relic Pack.
    pub fn open_relic_pack(ctx: Context<RequestArenaVrf>, client_seed: u8) -> Result<()> {
        let arena = &mut ctx.accounts.arena;
        require!(
            arena.phase == ArenaPhase::Idle as u8,
            BlitzError::RoundAlreadyActive
        );
        require!(
            arena.shards >= PACK_SHARD_COST,
            BlitzError::InsufficientShards
        );

        let now = Clock::get()?.unix_timestamp;
        let next_nonce = arena
            .round_nonce
            .checked_add(1)
            .ok_or(BlitzError::MathOverflow)?;

        arena.shards = arena
            .shards
            .checked_sub(PACK_SHARD_COST)
            .ok_or(BlitzError::MathOverflow)?;
        arena.round_nonce = next_nonce;
        arena.phase = ArenaPhase::AwaitingVrf as u8;
        arena.pending_vrf_kind = VrfKind::RelicPack as u8;
        arena.last_vrf_request_ts = now;

        let caller_seed = build_vrf_caller_seed(client_seed, next_nonce, now, VRF_DOMAIN_RELIC);

        let params = RequestRandomnessParams {
            payer: ctx.accounts.payer.key(),
            oracle_queue: ctx.accounts.oracle_queue.key(),
            callback_program_id: crate::ID,
            callback_discriminator: instruction::CallbackRelicVrf::DISCRIMINATOR.to_vec(),
            caller_seed,
            accounts_metas: Some(vec![SerializableAccountMeta {
                pubkey: arena.key(),
                is_signer: false,
                is_writable: true,
            }]),
            callback_args: Some(next_nonce.to_le_bytes().to_vec()),
            ..Default::default()
        };

        let ix = create_request_randomness_ix_const(params, PROGRAM_IDENTITY);
        ctx.accounts
            .invoke_signed_vrf(&ctx.accounts.payer.to_account_info(), &ix)?;

        msg!("Relic Pack VRF requested (nonce={})", next_nonce);
        Ok(())
    }

    /// Scoped VRF callback that mints a collectible Relic tier into the player's inventory.
    /// Drop table: Common 60% (+1%), Rare 25% (+3%), Epic 12% (+6%), Mythic 3% (+12%).
    pub fn callback_relic_vrf(
        ctx: Context<CallbackArenaVrf>,
        randomness: [u8; 32],
        expected_nonce: u64,
    ) -> Result<()> {
        let arena = &mut ctx.accounts.arena;
        require!(
            arena.phase == ArenaPhase::AwaitingVrf as u8
                && arena.pending_vrf_kind == VrfKind::RelicPack as u8
                && arena.round_nonce == expected_nonce,
            BlitzError::StaleVrfCallback
        );

        let roll = vrf::rnd::random_u8_with_range(&randomness, 1, 100);
        let tier = compute_relic_tier(roll);
        match tier {
            4 => arena.relics_mythic = arena.relics_mythic.saturating_add(1),
            3 => arena.relics_epic = arena.relics_epic.saturating_add(1),
            2 => arena.relics_rare = arena.relics_rare.saturating_add(1),
            _ => arena.relics_common = arena.relics_common.saturating_add(1),
        }

        arena.last_relic_tier = tier;
        arena.last_vrf_hash = randomness;
        arena.phase = ArenaPhase::Idle as u8;
        arena.pending_vrf_kind = VrfKind::None as u8;

        msg!(
            "Relic Pack opened (roll={}): minted tier {} (total boost={} bps)",
            roll,
            tier,
            arena.relic_boost_bps()
        );
        Ok(())
    }

    /// Safety recovery path: if a VRF oracle request times out (>125s), refund stake/shards
    /// and advance nonce so any late oracle callback is rejected.
    pub fn cancel_stale_vrf(ctx: Context<PlayArena>) -> Result<()> {
        let arena = &mut ctx.accounts.arena;
        require!(
            arena.phase == ArenaPhase::AwaitingVrf as u8,
            BlitzError::NoPendingVrf
        );

        let now = Clock::get()?.unix_timestamp;
        require!(
            is_vrf_expired(arena.last_vrf_request_ts, now),
            BlitzError::VrfNotExpiredYet
        );

        if arena.pending_vrf_kind == VrfKind::CrashRound as u8 {
            arena.chips = arena.chips.saturating_add(arena.active_stake);
            arena.active_stake = 0;
        } else if arena.pending_vrf_kind == VrfKind::RelicPack as u8 {
            arena.shards = arena.shards.saturating_add(PACK_SHARD_COST);
        }

        // Advance nonce so superseded callback fails closed
        arena.round_nonce = arena.round_nonce.saturating_add(1);
        arena.phase = ArenaPhase::Idle as u8;
        arena.pending_vrf_kind = VrfKind::None as u8;
        msg!("Stale VRF request cancelled & refunded safely");
        Ok(())
    }

    /// Checkpoint current ER ArenaState to Solana L1 without undelegating (SDK 0.18 MagicIntentBundleBuilder).
    pub fn commit_arena(ctx: Context<CommitArena>) -> Result<()> {
        MagicIntentBundleBuilder::new(
            ctx.accounts.payer.to_account_info(),
            ctx.accounts.magic_context.to_account_info(),
            ctx.accounts.magic_program.to_account_info(),
        )
        .commit(&[ctx.accounts.arena.to_account_info()])
        .build_and_invoke()?;
        Ok(())
    }

    /// Atomically commit final balances, streaks & Relic collection and undelegate back to Solana L1.
    pub fn undelegate_arena(ctx: Context<CommitArena>) -> Result<()> {
        require!(
            ctx.accounts.arena.phase == ArenaPhase::Idle as u8,
            BlitzError::RoundAlreadyActive
        );
        ctx.accounts.arena.exit(&crate::ID)?;
        MagicIntentBundleBuilder::new(
            ctx.accounts.payer.to_account_info(),
            ctx.accounts.magic_context.to_account_info(),
            ctx.accounts.magic_program.to_account_info(),
        )
        .commit_and_undelegate(&[ctx.accounts.arena.to_account_info()])
        .build_and_invoke()?;
        Ok(())
    }
}

/// Build a domain-separated 32-byte caller seed for MagicBlock VRF requests.
pub fn build_vrf_caller_seed(client_seed: u8, nonce: u64, ts: i64, domain_tag: u8) -> [u8; 32] {
    let mut caller_seed = [0u8; 32];
    caller_seed[0] = client_seed ^ domain_tag;
    caller_seed[1..9].copy_from_slice(&nonce.to_le_bytes());
    caller_seed[9..17].copy_from_slice(&ts.to_le_bytes());
    caller_seed[17] = domain_tag;
    caller_seed
}

/// Derive a heavy-tail crash multiplier in basis points (10_000 = 1.00x, cap 500_000 = 50.00x)
/// using standard 1% house-edge formula: M = floor(9900 / (1 - U)), with ~3% instant crash at 1.00x.
pub fn derive_crash_point_bps(randomness: &[u8; 32]) -> u32 {
    let mut raw_bytes = [0u8; 8];
    raw_bytes.copy_from_slice(&randomness[0..8]);
    let r = u64::from_le_bytes(raw_bytes);

    // ~3.03% instant crash (1.00x)
    if r % 33 == 0 {
        return 10_000;
    }

    // Map top 32 bits to u in [0, 9_999]
    let u = (r >> 32) % 10_000;
    let denom = 10_000u64.saturating_sub(u).max(1);
    let mult_bps = (99_000_000u64 / denom).clamp(MIN_CASHOUT_BPS as u64, MAX_CRASH_BPS as u64);
    mult_bps as u32
}

/// Compute (base_payout, bonus_profit, total_payout) with checked u128 arithmetic.
pub fn compute_payout(stake: u64, target_mult_bps: u32, boost_bps: u16) -> Result<(u64, u64, u64)> {
    let clamped_boost = boost_bps.min(MAX_RELIC_BOOST_BPS);
    let base_payout_u128 = (stake as u128)
        .checked_mul(target_mult_bps as u128)
        .ok_or(BlitzError::MathOverflow)?
        .checked_div(10_000)
        .ok_or(BlitzError::MathOverflow)?;
    let base_payout = u64::try_from(base_payout_u128).map_err(|_| BlitzError::MathOverflow)?;

    let raw_profit = base_payout.saturating_sub(stake);
    let bonus_profit_u128 = (raw_profit as u128)
        .checked_mul(clamped_boost as u128)
        .ok_or(BlitzError::MathOverflow)?
        .checked_div(10_000)
        .ok_or(BlitzError::MathOverflow)?;
    let bonus_profit = u64::try_from(bonus_profit_u128).map_err(|_| BlitzError::MathOverflow)?;

    let total_payout = base_payout
        .checked_add(bonus_profit)
        .ok_or(BlitzError::MathOverflow)?;
    Ok((base_payout, bonus_profit, total_payout))
}

/// Map a 1..=100 VRF roll to Relic tier: 4=Mythic(3%), 3=Epic(12%), 2=Rare(25%), 1=Common(60%).
pub fn compute_relic_tier(roll: u8) -> u8 {
    if roll <= 3 {
        4
    } else if roll <= 15 {
        3
    } else if roll <= 40 {
        2
    } else {
        1
    }
}

/// Check whether a pending VRF request has exceeded the 125-second timeout window.
pub fn is_vrf_expired(request_ts: i64, now_ts: i64) -> bool {
    now_ts.saturating_sub(request_ts) >= VRF_TIMEOUT_SECONDS
}

/// Derive the canonical ArenaState PDA for a given player pubkey.
pub fn derive_arena_pda(player: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[ARENA_SEED, player.as_ref()], &crate::ID)
}

#[repr(u8)]
pub enum ArenaPhase {
    Idle = 0,
    AwaitingVrf = 1,
    Live = 2,
}

#[repr(u8)]
pub enum VrfKind {
    None = 0,
    CrashRound = 1,
    RelicPack = 2,
}

#[account]
#[derive(InitSpace)]
pub struct ArenaState {
    pub player: Pubkey,
    pub chips: u64,
    pub shards: u64,
    pub round_nonce: u64,
    pub phase: u8,
    pub pending_vrf_kind: u8,
    pub active_stake: u64,
    pub crash_point_bps: u32,
    pub last_cashout_bps: u32,
    pub last_vrf_request_ts: i64,
    pub win_streak: u16,
    pub best_streak: u16,
    pub rounds_played: u32,
    pub total_pnl: i64,
    pub relics_common: u16,
    pub relics_rare: u16,
    pub relics_epic: u16,
    pub relics_mythic: u16,
    pub last_relic_tier: u8,
    pub last_vrf_hash: [u8; 32],
    pub bump: u8,
}

impl ArenaState {
    /// Total permanent profit boost from owned collectible Relics (capped at +25%).
    pub fn relic_boost_bps(&self) -> u16 {
        let raw = (self.relics_common as u32)
            .saturating_mul(100)
            .saturating_add((self.relics_rare as u32).saturating_mul(300))
            .saturating_add((self.relics_epic as u32).saturating_mul(600))
            .saturating_add((self.relics_mythic as u32).saturating_mul(1_200));
        raw.min(MAX_RELIC_BOOST_BPS as u32) as u16
    }
}

#[derive(Accounts)]
pub struct InitializeArena<'info> {
    #[account(
        init,
        payer = player,
        space = 8 + ArenaState::INIT_SPACE,
        seeds = [ARENA_SEED, player.key().as_ref()],
        bump
    )]
    pub arena: Account<'info, ArenaState>,
    #[account(mut)]
    pub player: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[delegate]
#[derive(Accounts)]
pub struct DelegateArena<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    /// CHECK: The ArenaState PDA to delegate to MagicBlock ER (UncheckedAccount required in SDK >=0.17.1)
    #[account(mut, del, seeds = [ARENA_SEED, payer.key().as_ref()], bump)]
    pub arena: UncheckedAccount<'info>,
}

#[vrf]
#[derive(Accounts)]
pub struct RequestArenaVrf<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        mut,
        seeds = [ARENA_SEED, payer.key().as_ref()],
        bump = arena.bump,
        constraint = arena.player == payer.key() @ BlitzError::Unauthorized
    )]
    pub arena: Account<'info, ArenaState>,
    /// CHECK: MagicBlock VRF Oracle Queue (base or ephemeral)
    #[account(
        mut,
        constraint =
            oracle_queue.key() == vrf::consts::DEFAULT_QUEUE ||
            oracle_queue.key() == vrf::consts::DEFAULT_EPHEMERAL_QUEUE ||
            oracle_queue.key() == vrf::consts::DEFAULT_TEST_QUEUE ||
            oracle_queue.key() == vrf::consts::DEFAULT_EPHEMERAL_TEST_QUEUE
            @ BlitzError::InvalidOracleQueue
    )]
    pub oracle_queue: UncheckedAccount<'info>,
}

#[vrf_callback]
#[derive(Accounts)]
pub struct CallbackArenaVrf<'info> {
    /// Scoped VRF oracle signer PDA bound to `crate::ID` (compile-time constant to keep callback CU minimal).
    #[account(address = SCOPED_VRF_IDENTITY)]
    pub vrf_program_identity: Signer<'info>,
    #[account(mut)]
    pub arena: Account<'info, ArenaState>,
}

#[derive(Accounts)]
pub struct PlayArena<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        mut,
        seeds = [ARENA_SEED, payer.key().as_ref()],
        bump = arena.bump,
        constraint = arena.player == payer.key() @ BlitzError::Unauthorized
    )]
    pub arena: Account<'info, ArenaState>,
}

#[commit]
#[derive(Accounts)]
pub struct CommitArena<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        mut,
        seeds = [ARENA_SEED, payer.key().as_ref()],
        bump = arena.bump,
        constraint = arena.player == payer.key() @ BlitzError::Unauthorized
    )]
    pub arena: Account<'info, ArenaState>,
}

#[error_code]
pub enum BlitzError {
    #[msg("Unauthorized player")]
    Unauthorized,
    #[msg("A round or VRF request is already active")]
    RoundAlreadyActive,
    #[msg("Stake is below minimum (100 chips)")]
    StakeTooLow,
    #[msg("Insufficient chips balance")]
    InsufficientChips,
    #[msg("Insufficient Vault Shards (need 100)")]
    InsufficientShards,
    #[msg("Round is not currently live")]
    RoundNotLive,
    #[msg("Target multiplier must be at least 1.05x (10,500 bps)")]
    InvalidMultiplier,
    #[msg("Invalid MagicBlock VRF oracle queue")]
    InvalidOracleQueue,
    #[msg("Stale or mismatched VRF callback")]
    StaleVrfCallback,
    #[msg("No pending VRF request to cancel")]
    NoPendingVrf,
    #[msg("VRF request has not expired yet (wait 125s)")]
    VrfNotExpiredYet,
    #[msg("Arithmetic overflow")]
    MathOverflow,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ephemeral_rollups_sdk::consts::{
        DELEGATION_PROGRAM_ID, MAGIC_CONTEXT_ID, MAGIC_PROGRAM_ID,
    };
    use std::collections::HashSet;

    fn sample_arena(player: Pubkey) -> ArenaState {
        ArenaState {
            player,
            chips: STARTING_CHIPS,
            shards: STARTING_SHARDS,
            round_nonce: 0,
            phase: ArenaPhase::Idle as u8,
            pending_vrf_kind: VrfKind::None as u8,
            active_stake: 0,
            crash_point_bps: 0,
            last_cashout_bps: 0,
            last_vrf_request_ts: 0,
            win_streak: 0,
            best_streak: 0,
            rounds_played: 0,
            total_pnl: 0,
            relics_common: 0,
            relics_rare: 0,
            relics_epic: 0,
            relics_mythic: 0,
            last_relic_tier: 0,
            last_vrf_hash: [0u8; 32],
            bump: 255,
        }
    }

    #[test]
    fn crash_point_bounds_and_edge_cases() {
        // All-zero seed triggers r % 33 == 0 -> instant crash 1.00x
        assert_eq!(derive_crash_point_bps(&[0u8; 32]), 10_000);

        // Multiple of 33 in low 64 bits -> instant crash
        let mut mul33 = [0u8; 32];
        mul33[0..8].copy_from_slice(&(33u64 * 999).to_le_bytes());
        assert_eq!(derive_crash_point_bps(&mul33), 10_000);

        // u = 9_999 (denom = 1) -> hits 50.00x cap (500_000 bps)
        let mut max_cap = [0u8; 32];
        let val = (9_999u64 << 32) | 1u64; // ensure % 33 != 0
        assert_ne!(val % 33, 0);
        max_cap[0..8].copy_from_slice(&val.to_le_bytes());
        assert_eq!(derive_crash_point_bps(&max_cap), MAX_CRASH_BPS);

        // u = 0 (denom = 10_000) -> 9_900 clamped up to 10_500 (1.05x floor for non-instant rounds)
        let mut floor_seed = [0u8; 32];
        floor_seed[0..8].copy_from_slice(&1u64.to_le_bytes());
        assert_eq!(derive_crash_point_bps(&floor_seed), MIN_CASHOUT_BPS);

        // All-FF seed must remain cleanly within bounds
        let all_ff = [0xFFu8; 32];
        let m_ff = derive_crash_point_bps(&all_ff);
        assert!(m_ff == 10_000 || (MIN_CASHOUT_BPS..=MAX_CRASH_BPS).contains(&m_ff));
    }

    #[test]
    fn crash_point_statistical_heavy_tail_distribution() {
        let mut instant = 0u32;
        let mut over_2x = 0u32;
        let mut over_10x = 0u32;
        let total = 5_000u32;

        for i in 0..total {
            // Simple deterministic splitmix64 PRNG for test vectors
            let mut x = (i as u64).wrapping_add(0x9E37_79B9_7F4A_7C15);
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            x = x ^ (x >> 31);
            let mut seed = [0u8; 32];
            seed[0..8].copy_from_slice(&x.to_le_bytes());
            let m = derive_crash_point_bps(&seed);
            assert!(m == 10_000 || (MIN_CASHOUT_BPS..=MAX_CRASH_BPS).contains(&m));
            if m == 10_000 {
                instant += 1;
            }
            if m >= 20_000 {
                over_2x += 1;
            }
            if m >= 100_000 {
                over_10x += 1;
            }
        }

        // Expect ~3% instant crashes (~150/5000), ~48% >= 2x, ~9% >= 10x
        assert!((100..=210).contains(&instant), "instant={instant}");
        assert!((2100..=2700).contains(&over_2x), "over_2x={over_2x}");
        assert!((350..=620).contains(&over_10x), "over_10x={over_10x}");
    }

    #[test]
    fn vrf_caller_seed_domain_separation_and_determinism() {
        let s_round_a = build_vrf_caller_seed(42, 1, 1_700_000_000, VRF_DOMAIN_ROUND);
        let s_round_b = build_vrf_caller_seed(42, 1, 1_700_000_000, VRF_DOMAIN_ROUND);
        let s_relic = build_vrf_caller_seed(42, 1, 1_700_000_000, VRF_DOMAIN_RELIC);
        let s_next_nonce = build_vrf_caller_seed(42, 2, 1_700_000_000, VRF_DOMAIN_ROUND);
        let s_next_ts = build_vrf_caller_seed(42, 1, 1_700_000_001, VRF_DOMAIN_ROUND);

        assert_eq!(s_round_a, s_round_b);
        assert_ne!(s_round_a, s_relic, "round and relic VRF domains must differ");
        assert_ne!(s_round_a, s_next_nonce, "nonce must perturb VRF seed");
        assert_ne!(s_round_a, s_next_ts, "timestamp must perturb VRF seed");
        assert_eq!(s_round_a[17], VRF_DOMAIN_ROUND);
        assert_eq!(s_relic[17], VRF_DOMAIN_RELIC);
    }

    #[test]
    fn relic_boost_scales_caps_and_never_overflows() {
        let mut arena = sample_arena(Pubkey::default());
        assert_eq!(arena.relic_boost_bps(), 0);

        arena.relics_common = 2; // 200
        arena.relics_rare = 1;   // 300
        arena.relics_epic = 1;   // 600
        assert_eq!(arena.relic_boost_bps(), 1_100);

        arena.relics_mythic = 1; // +1200 -> 2300
        assert_eq!(arena.relic_boost_bps(), 2_300);

        arena.relics_mythic = 5; // uncapped 7100 -> capped at 2500
        assert_eq!(arena.relic_boost_bps(), MAX_RELIC_BOOST_BPS);

        // Extreme u16::MAX overflow resilience
        arena.relics_common = u16::MAX;
        arena.relics_rare = u16::MAX;
        arena.relics_epic = u16::MAX;
        arena.relics_mythic = u16::MAX;
        assert_eq!(arena.relic_boost_bps(), MAX_RELIC_BOOST_BPS);
    }

    #[test]
    fn relic_drop_table_boundaries_exact() {
        for roll in 1..=3 {
            assert_eq!(compute_relic_tier(roll), 4, "roll {roll} must be Mythic");
        }
        for roll in 4..=15 {
            assert_eq!(compute_relic_tier(roll), 3, "roll {roll} must be Epic");
        }
        for roll in 16..=40 {
            assert_eq!(compute_relic_tier(roll), 2, "roll {roll} must be Rare");
        }
        for roll in 41..=100 {
            assert_eq!(compute_relic_tier(roll), 1, "roll {roll} must be Common");
        }
    }

    #[test]
    fn payout_solvency_and_relic_boost_math() {
        // 1,000 stake @ 2.00x with 0% boost -> base 2,000, bonus 0, total 2,000
        let (base, bonus, total) = compute_payout(1_000, 20_000, 0).unwrap();
        assert_eq!((base, bonus, total), (2_000, 0, 2_000));

        // 1,000 stake @ 2.00x with +25% max Relic boost -> profit 1,000 * 25% = +250 -> total 2,250
        let (base, bonus, total) = compute_payout(1_000, 20_000, 2_500).unwrap();
        assert_eq!((base, bonus, total), (2_000, 250, 2_250));

        // Boost above MAX_RELIC_BOOST_BPS is clamped defensively inside compute_payout
        let (_, bonus_clamped, total_clamped) = compute_payout(1_000, 20_000, 9_999).unwrap();
        assert_eq!((bonus_clamped, total_clamped), (250, 2_250));

        // 50.00x max crash payout with 10,000 stake and max Relic boost
        let (base_50, bonus_50, total_50) = compute_payout(10_000, MAX_CRASH_BPS, 2_500).unwrap();
        assert_eq!(base_50, 500_000);
        assert_eq!(bonus_50, 122_500); // 25% of 490_000 raw profit
        assert_eq!(total_50, 622_500);

        // Solvency bound: for any stake >= MIN_STAKE and mult >= MIN_CASHOUT_BPS, total >= base >= stake
        for stake in [100u64, 500, 10_000, 1_000_000] {
            for mult in [MIN_CASHOUT_BPS, 15_000, 35_000, MAX_CRASH_BPS] {
                let (b, bo, t) = compute_payout(stake, mult, 1_200).unwrap();
                assert!(b >= stake && t == b + bo && t >= stake);
            }
        }

        // Overflow guard on u64::MAX stake
        assert!(compute_payout(u64::MAX, MAX_CRASH_BPS, 2_500).is_err());
    }

    #[test]
    fn arena_pda_derivation_is_unique_and_canonical() {
        let p1 = Pubkey::new_from_array([1u8; 32]);
        let p2 = Pubkey::new_from_array([2u8; 32]);
        let (pda1, bump1) = derive_arena_pda(&p1);
        let (pda2, bump2) = derive_arena_pda(&p2);
        assert_ne!(pda1, pda2);
        assert_eq!(
            Pubkey::create_program_address(&[ARENA_SEED, p1.as_ref(), &[bump1]], &crate::ID)
                .unwrap(),
            pda1
        );
        assert_eq!(
            Pubkey::create_program_address(&[ARENA_SEED, p2.as_ref(), &[bump2]], &crate::ID)
                .unwrap(),
            pda2
        );
    }

    #[test]
    fn arena_state_borsh_layout_matches_init_space() {
        let player = Pubkey::new_from_array([7u8; 32]);
        let arena = sample_arena(player);
        let mut buf = Vec::new();
        arena.try_serialize(&mut buf).unwrap();
        assert_eq!(
            buf.len(),
            8 + ArenaState::INIT_SPACE,
            "Serialized ArenaState length must match 8 + INIT_SPACE"
        );
        let decoded = ArenaState::try_deserialize(&mut buf.as_slice()).unwrap();
        assert_eq!(decoded.player, player);
        assert_eq!(decoded.chips, STARTING_CHIPS);
        assert_eq!(decoded.shards, STARTING_SHARDS);
    }

    #[test]
    fn magicblock_er_and_vrf_program_ids_verified() {
        // Verify canonical MagicBlock Delegation Program, Magic Program, Magic Context & VRF Queues
        assert_eq!(
            DELEGATION_PROGRAM_ID.to_string(),
            "DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh"
        );
        assert_eq!(
            MAGIC_PROGRAM_ID.to_string(),
            "Magic11111111111111111111111111111111111111"
        );
        assert_eq!(
            MAGIC_CONTEXT_ID.to_string(),
            "MagicContext1111111111111111111111111111111"
        );
        assert_eq!(
            vrf::consts::DEFAULT_EPHEMERAL_QUEUE.to_string(),
            "5hBR571xnXppuCPveTrctfTU7tJLSN94nq7kv7FRK5Tc"
        );

        // Compile-time PROGRAM_IDENTITY must equal runtime PDA [b"identity"] under crate::ID
        let (expected_identity, _) = Pubkey::find_program_address(&[b"identity"], &crate::ID);
        assert_eq!(PROGRAM_IDENTITY, expected_identity);

        // Compile-time SCOPED_VRF_IDENTITY must equal runtime PDA [b"identity", crate::ID] under VRF_PROGRAM_ID
        let (expected_scoped, _) = Pubkey::find_program_address(
            &[b"identity", crate::ID.as_ref()],
            &vrf::consts::VRF_PROGRAM_ID,
        );
        assert_eq!(SCOPED_VRF_IDENTITY, expected_scoped);
    }

    #[test]
    fn magicblock_vrf_request_instruction_layouts() {
        let payer = Pubkey::new_from_array([9u8; 32]);
        let (arena_pda, _) = derive_arena_pda(&payer);

        for (disc, domain) in [
            (instruction::CallbackRoundVrf::DISCRIMINATOR, VRF_DOMAIN_ROUND),
            (instruction::CallbackRelicVrf::DISCRIMINATOR, VRF_DOMAIN_RELIC),
        ] {
            let caller_seed = build_vrf_caller_seed(17, 3, 1_760_000_000, domain);
            let params = RequestRandomnessParams {
                payer,
                oracle_queue: vrf::consts::DEFAULT_EPHEMERAL_QUEUE,
                callback_program_id: crate::ID,
                callback_discriminator: disc.to_vec(),
                caller_seed,
                accounts_metas: Some(vec![SerializableAccountMeta {
                    pubkey: arena_pda,
                    is_signer: false,
                    is_writable: true,
                }]),
                callback_args: Some(3u64.to_le_bytes().to_vec()),
                ..Default::default()
            };
            let ix = create_request_randomness_ix_const(params, PROGRAM_IDENTITY);
            assert!(
                ix.accounts.iter().any(|m| m.pubkey == payer && m.is_signer),
                "payer must be signer in VRF request ix"
            );
            assert!(
                ix.accounts
                    .iter()
                    .any(|m| m.pubkey == vrf::consts::DEFAULT_EPHEMERAL_QUEUE && m.is_writable),
                "ephemeral oracle queue must be writable"
            );
            assert!(
                ix.accounts.iter().any(|m| m.pubkey == PROGRAM_IDENTITY),
                "scoped PROGRAM_IDENTITY must be included in VRF request ix"
            );
            assert!(!ix.data.is_empty());
        }
    }

    #[test]
    fn magicblock_delegation_pda_layouts() {
        let player = Pubkey::new_from_array([11u8; 32]);
        let (arena_pda, _) = derive_arena_pda(&player);
        let (buffer_pda, _) =
            Pubkey::find_program_address(&[b"buffer", arena_pda.as_ref()], &crate::ID);
        let (delegation_record, _) = Pubkey::find_program_address(
            &[b"delegation", arena_pda.as_ref()],
            &DELEGATION_PROGRAM_ID,
        );
        let (delegation_metadata, _) = Pubkey::find_program_address(
            &[b"delegation-metadata", arena_pda.as_ref()],
            &DELEGATION_PROGRAM_ID,
        );
        let unique: HashSet<Pubkey> = [
            arena_pda,
            buffer_pda,
            delegation_record,
            delegation_metadata,
        ]
        .into_iter()
        .collect();
        assert_eq!(unique.len(), 4, "all delegation PDAs must be distinct");
    }

    #[test]
    fn all_instruction_discriminators_are_unique() {
        let discs: Vec<&[u8]> = vec![
            instruction::InitializeArena::DISCRIMINATOR,
            instruction::DelegateArena::DISCRIMINATOR,
            instruction::StartRound::DISCRIMINATOR,
            instruction::CallbackRoundVrf::DISCRIMINATOR,
            instruction::CashoutRound::DISCRIMINATOR,
            instruction::OpenRelicPack::DISCRIMINATOR,
            instruction::CallbackRelicVrf::DISCRIMINATOR,
            instruction::CancelStaleVrf::DISCRIMINATOR,
            instruction::CommitArena::DISCRIMINATOR,
            instruction::UndelegateArena::DISCRIMINATOR,
        ];
        let set: HashSet<&[u8]> = discs.iter().copied().collect();
        assert_eq!(set.len(), 10, "all 10 Anchor instructions must have unique discriminators");
    }

    #[test]
    fn stale_vrf_timeout_boundary_and_nonce_guard() {
        let t0 = 1_760_000_000i64;
        assert!(!is_vrf_expired(t0, t0));
        assert!(!is_vrf_expired(t0, t0 + VRF_TIMEOUT_SECONDS - 1));
        assert!(is_vrf_expired(t0, t0 + VRF_TIMEOUT_SECONDS));
        assert!(is_vrf_expired(t0, t0 + VRF_TIMEOUT_SECONDS + 60));
        // Clock skew backward must never falsely expire
        assert!(!is_vrf_expired(t0, t0 - 10));
    }

    #[test]
    fn full_gameplay_lifecycle_simulation() {
        let player = Pubkey::new_from_array([33u8; 32]);
        let mut arena = sample_arena(player);

        // 1. Open introductory Relic Pack (150 -> 50 shards), simulate Epic roll (roll = 10 -> +6%)
        arena.shards -= PACK_SHARD_COST;
        let tier = compute_relic_tier(10);
        assert_eq!(tier, 3);
        arena.relics_epic += 1;
        assert_eq!(arena.relic_boost_bps(), 600);

        // 2. Round 1: Stake 1,000, crash = 3.00x, cash out @ 2.00x -> WIN + 6% relic boost on profit
        arena.chips -= 1_000;
        let (_, bonus, total) = compute_payout(1_000, 20_000, arena.relic_boost_bps()).unwrap();
        assert_eq!(bonus, 60);
        assert_eq!(total, 2_060);
        arena.chips += total;
        arena.win_streak += 1;
        arena.shards += 25 + (arena.win_streak as u64) * 10; // +35 shards -> 85

        assert_eq!(arena.chips, 11_060);
        assert_eq!(arena.shards, 85);
        assert_eq!(arena.win_streak, 1);

        // 3. Round 2: Bust -> streak resets, +10 consolation shards
        arena.chips -= 500;
        arena.win_streak = 0;
        arena.shards += 10;
        assert_eq!(arena.chips, 10_560);
        assert_eq!(arena.shards, 95);
    }
}
