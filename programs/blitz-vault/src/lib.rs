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
const PROGRAM_IDENTITY: Pubkey = program_identity_pubkey(&crate::ID);

pub const ARENA_SEED: &[u8] = b"blitz_arena_v1";
pub const STARTING_CHIPS: u64 = 10_000;
pub const MIN_STAKE: u64 = 100;
pub const PACK_SHARD_COST: u64 = 100;
pub const VRF_TIMEOUT_SECONDS: i64 = 125;
pub const MAX_RELIC_BOOST_BPS: u16 = 2_500; // +25.00% max yield boost

#[ephemeral]
#[program]
pub mod blitz_vault {
    use super::*;

    /// Initialize a player's ArenaState PDA on Solana Base Layer with starter chips.
    pub fn initialize_arena(ctx: Context<InitializeArena>) -> Result<()> {
        let arena = &mut ctx.accounts.arena;
        arena.player = ctx.accounts.player.key();
        arena.chips = STARTING_CHIPS;
        arena.shards = 150; // Enough for 1 introductory Relic Pack opening
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
        let next_nonce = arena.round_nonce.checked_add(1).ok_or(BlitzError::MathOverflow)?;

        arena.chips = arena.chips.checked_sub(stake).ok_or(BlitzError::MathOverflow)?;
        arena.active_stake = stake;
        arena.round_nonce = next_nonce;
        arena.phase = ArenaPhase::AwaitingVrf as u8;
        arena.pending_vrf_kind = VrfKind::CrashRound as u8;
        arena.crash_point_bps = 0;
        arena.last_vrf_request_ts = now;

        let mut caller_seed = [0u8; 32];
        caller_seed[0] = client_seed;
        caller_seed[1..9].copy_from_slice(&next_nonce.to_le_bytes());
        caller_seed[9..17].copy_from_slice(&now.to_le_bytes());

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
    pub fn cashout_round(
        ctx: Context<PlayArena>,
        target_mult_bps: u32,
    ) -> Result<()> {
        let arena = &mut ctx.accounts.arena;
        require!(
            arena.phase == ArenaPhase::Live as u8,
            BlitzError::RoundNotLive
        );
        require!(target_mult_bps >= 10_500, BlitzError::InvalidMultiplier); // min 1.05x

        let stake = arena.active_stake;
        arena.rounds_played = arena.rounds_played.saturating_add(1);
        arena.last_cashout_bps = target_mult_bps;

        if target_mult_bps <= arena.crash_point_bps {
            // Base payout = stake * target_mult_bps / 10_000
            let base_payout = (stake as u128)
                .checked_mul(target_mult_bps as u128)
                .ok_or(BlitzError::MathOverflow)?
                .checked_div(10_000)
                .ok_or(BlitzError::MathOverflow)? as u64;

            // Profit gets boosted by owned Collectible Relics (up to +25%)
            let raw_profit = base_payout.saturating_sub(stake);
            let boost_bps = arena.relic_boost_bps();
            let bonus_profit = (raw_profit as u128)
                .checked_mul(boost_bps as u128)
                .ok_or(BlitzError::MathOverflow)?
                .checked_div(10_000)
                .ok_or(BlitzError::MathOverflow)? as u64;

            let total_payout = base_payout
                .checked_add(bonus_profit)
                .ok_or(BlitzError::MathOverflow)?;
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
    pub fn open_relic_pack(
        ctx: Context<RequestArenaVrf>,
        client_seed: u8,
    ) -> Result<()> {
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
        let next_nonce = arena.round_nonce.checked_add(1).ok_or(BlitzError::MathOverflow)?;

        arena.shards = arena
            .shards
            .checked_sub(PACK_SHARD_COST)
            .ok_or(BlitzError::MathOverflow)?;
        arena.round_nonce = next_nonce;
        arena.phase = ArenaPhase::AwaitingVrf as u8;
        arena.pending_vrf_kind = VrfKind::RelicPack as u8;
        arena.last_vrf_request_ts = now;

        let mut caller_seed = [0u8; 32];
        caller_seed[0] = client_seed ^ 0xA5;
        caller_seed[1..9].copy_from_slice(&next_nonce.to_le_bytes());

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
        let tier = if roll <= 3 {
            arena.relics_mythic = arena.relics_mythic.saturating_add(1);
            4u8 // Mythic
        } else if roll <= 15 {
            arena.relics_epic = arena.relics_epic.saturating_add(1);
            3u8 // Epic
        } else if roll <= 40 {
            arena.relics_rare = arena.relics_rare.saturating_add(1);
            2u8 // Rare
        } else {
            arena.relics_common = arena.relics_common.saturating_add(1);
            1u8 // Common
        };

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
            now.saturating_sub(arena.last_vrf_request_ts) >= VRF_TIMEOUT_SECONDS,
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

/// Derive a heavy-tail crash multiplier in basis points (10_000 = 1.00x, cap 500_000 = 50.00x)
/// using standard 1% house-edge formula: M = floor(9900 / (1 - U)), with 3% instant crash at 1.00x.
fn derive_crash_point_bps(randomness: &[u8; 32]) -> u32 {
    let mut raw_bytes = [0u8; 8];
    raw_bytes.copy_from_slice(&randomness[0..8]);
    let r = u64::from_le_bytes(raw_bytes);

    // 3% instant crash (1.00x)
    if r % 33 == 0 {
        return 10_000;
    }

    // Map top 32 bits to u in [0, 9_999]
    let u = (r >> 32) % 10_000;
    let denom = 10_000u64.saturating_sub(u).max(1);
    let mult_bps = (99_000_000u64 / denom).clamp(10_500, 500_000);
    mult_bps as u32
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
        let raw = (self.relics_common as u32) * 100
            + (self.relics_rare as u32) * 300
            + (self.relics_epic as u32) * 600
            + (self.relics_mythic as u32) * 1_200;
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

    #[test]
    fn crash_point_bounds_and_instant_crash() {
        let zero = [0u8; 32];
        assert_eq!(derive_crash_point_bps(&zero), 10_000);

        for i in 1u8..=200 {
            let mut seed = [0u8; 32];
            seed[0] = i;
            seed[4] = i.wrapping_mul(37);
            seed[5] = i.wrapping_mul(113);
            let m = derive_crash_point_bps(&seed);
            assert!(
                m == 10_000 || (10_500..=500_000).contains(&m),
                "unexpected crash multiplier: {m}"
            );
        }
    }

    #[test]
    fn relic_boost_scales_and_caps() {
        let mut arena = ArenaState {
            player: Pubkey::default(),
            chips: STARTING_CHIPS,
            shards: 0,
            round_nonce: 0,
            phase: 0,
            pending_vrf_kind: 0,
            active_stake: 0,
            crash_point_bps: 0,
            last_cashout_bps: 0,
            last_vrf_request_ts: 0,
            win_streak: 0,
            best_streak: 0,
            rounds_played: 0,
            total_pnl: 0,
            relics_common: 2,
            relics_rare: 1,
            relics_epic: 1,
            relics_mythic: 0,
            last_relic_tier: 0,
            last_vrf_hash: [0u8; 32],
            bump: 0,
        };
        assert_eq!(arena.relic_boost_bps(), 1_100);
        arena.relics_mythic = 5;
        assert_eq!(arena.relic_boost_bps(), MAX_RELIC_BOOST_BPS);
    }
}
