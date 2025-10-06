use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("FtQbMDA7w8a9icfbMkuTxxQ695Wp9e6RQFSGVjmYQgz3");

// =================================================================
// ⭐️ DATA ACCOUNTS (MUST BE BEFORE #[program] MODULE) ⭐️
// =================================================================

#[account]
#[derive(InitSpace)]
pub struct BettingRound {
    pub round_id: u64,
    pub authority: Pubkey,
    pub treasury: Pubkey,
    pub betting_start_time: i64,
    pub betting_end_time: i64,
    pub fight_end_time: i64,
    pub initial_hp: u32,
    pub current_hp: u32,
    pub fee_percentage: u8,
    pub phase: GamePhase,
    pub total_death_bets: u64,
    pub total_survival_bets: u64,
    pub total_bets_count: u64,
    pub boss_defeated: bool,
    pub payouts_processed: bool,
    pub escrow_bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct BetAccount {
    pub bettor: Pubkey,
    pub round_id: u64,
    pub amount: u64,
    pub prediction: BossPrediction,
    #[max_len(32)]
    pub username: String,
    pub timestamp: i64,
    pub payout_claimed: bool,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq, InitSpace)]
pub enum GamePhase {
    Betting,
    Fighting,
    Ended,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq, InitSpace)]
pub enum BossPrediction {
    Death,
    Survival,
}

// =================================================================
// ✅ EVENTS (MOVED HERE TO FIX SCOPING ERROR E0422) ✅
// =================================================================

#[event]
pub struct BettingRoundInitialized {
    pub round_id: u64,
    pub betting_end_time: i64,
    pub fight_end_time: i64,
}

#[event]
pub struct BetPlaced {
    pub round_id: u64,
    pub bettor: Pubkey,
    pub amount: u64,
    pub prediction: BossPrediction,
    pub username: String,
}

#[event]
pub struct FightPhaseStarted {
    pub round_id: u64,
    pub fight_end_time: i64,
}

#[event]
pub struct BossHpUpdated {
    pub round_id: u64,
    pub new_hp: u32,
}

#[event]
pub struct FightEnded {
    pub round_id: u64,
    pub boss_defeated: bool,
}

#[event]
pub struct PayoutClaimed {
    pub round_id: u64,
    pub bettor: Pubkey,
    pub original_bet: u64,
    pub prize_share: u64,
    pub total_payout: u64,
}

#[event]
pub struct FeesClaimed {
    pub round_id: u64,
    pub treasury: Pubkey,
    pub amount: u64,
}

// =================================================================
// ⭐️ PROGRAM INSTRUCTIONS ⭐️
// =================================================================
#[program]
pub mod boss_fight_betting {
    use super::*;

    /// Initialize a new betting round
    pub fn initialize_betting_round(
        ctx: Context<InitializeBettingRound>,
        round_id: u64,
        betting_duration: i64,
        fight_duration: i64,
        initial_hp: u32,
        fee_percentage: u8,
    ) -> Result<()> {
        let betting_round = &mut ctx.accounts.betting_round;
        let clock = Clock::get()?;

        betting_round.round_id = round_id;
        betting_round.authority = ctx.accounts.authority.key();
        betting_round.treasury = ctx.accounts.treasury.key();
        betting_round.betting_start_time = clock.unix_timestamp;
        
        // FIXED: Use checked_add for arithmetic operations
        betting_round.betting_end_time = clock.unix_timestamp
            .checked_add(betting_duration)
            .ok_or(BettingError::ArithmeticOverflow)?;
        
        betting_round.fight_end_time = clock.unix_timestamp
            .checked_add(betting_duration)
            .ok_or(BettingError::ArithmeticOverflow)?
            .checked_add(fight_duration)
            .ok_or(BettingError::ArithmeticOverflow)?;
        
        betting_round.initial_hp = initial_hp;
        betting_round.current_hp = initial_hp;
        betting_round.fee_percentage = fee_percentage;
        betting_round.phase = GamePhase::Betting;
        betting_round.total_death_bets = 0;
        betting_round.total_survival_bets = 0;
        betting_round.total_bets_count = 0;
        betting_round.boss_defeated = false;
        betting_round.payouts_processed = false;
        betting_round.escrow_bump = ctx.bumps.escrow;

        emit!(BettingRoundInitialized {
            round_id,
            betting_end_time: betting_round.betting_end_time,
            fight_end_time: betting_round.fight_end_time,
        });

        Ok(())
    }

    /// Place a bet on boss death or survival
    pub fn place_bet(
        ctx: Context<PlaceBet>,
        amount: u64,
        prediction: BossPrediction,
        username: String,
    ) -> Result<()> {
        let betting_round = &mut ctx.accounts.betting_round;
        let bet_account = &mut ctx.accounts.bet_account;
        let clock = Clock::get()?;

        // Validate betting phase and timing
        require!(
            betting_round.phase == GamePhase::Betting,
            BettingError::NotInBettingPhase
        );
        require!(
            clock.unix_timestamp <= betting_round.betting_end_time,
            BettingError::BettingPeriodExpired
        );
        require!(amount >= 100_000_000, BettingError::BetTooSmall); // Minimum 0.1 SOL
        require!(username.len() <= 32, BettingError::UsernameTooLong);

        // Transfer SOL to escrow
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.bettor.to_account_info(),
                    to: ctx.accounts.escrow.to_account_info(),
                },
            ),
            amount,
        )?;

        // Initialize bet account
        bet_account.bettor = ctx.accounts.bettor.key();
        bet_account.round_id = betting_round.round_id;
        bet_account.amount = amount;
        bet_account.prediction = prediction.clone();
        bet_account.username = username;
        bet_account.timestamp = clock.unix_timestamp;
        bet_account.payout_claimed = false;

        // Update betting round totals
        match prediction {
            BossPrediction::Death => betting_round.total_death_bets += amount,
            BossPrediction::Survival => betting_round.total_survival_bets += amount,
        }
        betting_round.total_bets_count += 1;

        emit!(BetPlaced {
            round_id: betting_round.round_id,
            bettor: ctx.accounts.bettor.key(),
            amount,
            prediction: prediction.clone(),
            username: bet_account.username.clone(),
        });

        Ok(())
    }

    /// Start the fighting phase (called by authority after betting ends)
    pub fn start_fight_phase(ctx: Context<StartFightPhase>) -> Result<()> {
        let betting_round = &mut ctx.accounts.betting_round;
        let clock = Clock::get()?;

        require!(
            betting_round.phase == GamePhase::Betting,
            BettingError::NotInBettingPhase
        );
        require!(
            clock.unix_timestamp >= betting_round.betting_end_time,
            BettingError::BettingStillActive
        );
        require!(
            ctx.accounts.authority.key() == betting_round.authority,
            BettingError::Unauthorized
        );

        betting_round.phase = GamePhase::Fighting;

        emit!(FightPhaseStarted {
            round_id: betting_round.round_id,
            fight_end_time: betting_round.fight_end_time,
        });

        Ok(())
    }

    /// Update boss HP (called by authority during fight)
    pub fn update_boss_hp(ctx: Context<UpdateBossHp>, new_hp: u32) -> Result<()> {
        let betting_round = &mut ctx.accounts.betting_round;
        let clock = Clock::get()?;

        require!(
            betting_round.phase == GamePhase::Fighting,
            BettingError::NotInFightPhase
        );
        require!(
            clock.unix_timestamp <= betting_round.fight_end_time,
            BettingError::FightPeriodExpired
        );
        require!(
            ctx.accounts.authority.key() == betting_round.authority,
            BettingError::Unauthorized
        );

        betting_round.current_hp = new_hp;

        emit!(BossHpUpdated {
            round_id: betting_round.round_id,
            new_hp,
        });

        Ok(())
    }

    /// End the fight and determine outcome
    pub fn end_fight(ctx: Context<EndFight>) -> Result<()> {
        let betting_round = &mut ctx.accounts.betting_round;
        let clock = Clock::get()?;

        require!(
            betting_round.phase == GamePhase::Fighting,
            BettingError::NotInFightPhase
        );
        require!(
            ctx.accounts.authority.key() == betting_round.authority,
            BettingError::Unauthorized
        );

        // Fight can end either by time expiry or boss HP reaching 0
        let fight_expired = clock.unix_timestamp >= betting_round.fight_end_time;
        let boss_dead = betting_round.current_hp == 0;

        require!(fight_expired || boss_dead, BettingError::FightNotFinished);

        betting_round.phase = GamePhase::Ended;
        betting_round.boss_defeated = boss_dead;

        emit!(FightEnded {
            round_id: betting_round.round_id,
            boss_defeated: boss_dead,
        });

        Ok(())
    }

    /// Claim payout for winning bet
    pub fn claim_payout(ctx: Context<ClaimPayout>) -> Result<()> {
        let betting_round = &ctx.accounts.betting_round;
        let bet_account = &mut ctx.accounts.bet_account;
        let escrow_info = ctx.accounts.escrow.to_account_info();

        require!(
            betting_round.phase == GamePhase::Ended,
            BettingError::FightNotEnded
        );
        require!(
            !bet_account.payout_claimed,
            BettingError::PayoutAlreadyClaimed
        );
        require!(
            bet_account.bettor == ctx.accounts.bettor.key(),
            BettingError::Unauthorized
        );

        // Check if bet won
        let won = match bet_account.prediction {
            BossPrediction::Death => betting_round.boss_defeated,
            BossPrediction::Survival => !betting_round.boss_defeated,
        };

        require!(won, BettingError::BetLost);

        // Calculate payout
        let total_winning_bets = if betting_round.boss_defeated {
            betting_round.total_death_bets
        } else {
            betting_round.total_survival_bets
        };

        let total_losing_bets = if betting_round.boss_defeated {
            betting_round.total_survival_bets
        } else {
            betting_round.total_death_bets
        };

        let fee_amount = total_losing_bets
            .checked_mul(betting_round.fee_percentage as u64)
            .unwrap()
            .checked_div(100)
            .unwrap();

        let prize_pool = total_losing_bets.checked_sub(fee_amount).unwrap();

        // Calculate individual payout
        let prize_share = if total_winning_bets > 0 {
            prize_pool
                .checked_mul(bet_account.amount)
                .unwrap()
                .checked_div(total_winning_bets)
                .unwrap()
        } else {
            0
        };

        let total_payout = bet_account.amount.checked_add(prize_share).unwrap();
        
        // Ensure escrow has enough lamports to send the payout
        require!(escrow_info.lamports() >= total_payout, BettingError::InsufficientEscrowFunds);

        // Prepare the PDA seeds for signing
        let round_id_bytes = betting_round.round_id.to_le_bytes();
        let escrow_seeds: &[&[u8]] = &[
            b"escrow",
            round_id_bytes.as_ref(),
            &[betting_round.escrow_bump],
        ];
        let signer_seeds = &[&escrow_seeds[..]];

        // Execute the transfer, signed by the PDA
        system_program::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: escrow_info,
                    to: ctx.accounts.bettor.to_account_info(),
                },
                signer_seeds
            ),
            total_payout,
        )?;

        bet_account.payout_claimed = true;

        emit!(PayoutClaimed {
            round_id: betting_round.round_id,
            bettor: ctx.accounts.bettor.key(),
            original_bet: bet_account.amount,
            prize_share,
            total_payout,
        });
        
        Ok(())
    }

    /// Claim fees (called by treasury)
    pub fn claim_fees(ctx: Context<ClaimFees>) -> Result<()> {
        let betting_round = &ctx.accounts.betting_round;
        let escrow_info = ctx.accounts.escrow.to_account_info();

        require!(
            betting_round.phase == GamePhase::Ended,
            BettingError::FightNotEnded
        );
        require!(
            ctx.accounts.treasury.key() == betting_round.treasury,
            BettingError::Unauthorized
        );

        let total_losing_bets = if betting_round.boss_defeated {
            betting_round.total_survival_bets
        } else {
            betting_round.total_death_bets
        };

        let fee_amount = total_losing_bets
            .checked_mul(betting_round.fee_percentage as u64)
            .unwrap()
            .checked_div(100)
            .unwrap();

        // Ensure escrow has enough lamports to send the fees
        require!(escrow_info.lamports() >= fee_amount, BettingError::InsufficientEscrowFunds);

        // Prepare the PDA seeds for signing
        let round_id_bytes = betting_round.round_id.to_le_bytes();
        let escrow_seeds: &[&[u8]] = &[
            b"escrow",
            round_id_bytes.as_ref(),
            &[betting_round.escrow_bump],
        ];
        let signer_seeds = &[&escrow_seeds[..]];

        // Execute the transfer, signed by the PDA
        system_program::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: escrow_info,
                    to: ctx.accounts.treasury.to_account_info(),
                },
                signer_seeds
            ),
            fee_amount,
        )?;

        emit!(FeesClaimed {
            round_id: betting_round.round_id,
            treasury: ctx.accounts.treasury.key(),
            amount: fee_amount,
        });

        Ok(())
    }
}

// =================================================================
// ⭐️ ACCOUNTS CONTEXTS (UPDATED FOR BUMPS & WRITABILITY) ⭐️
// =================================================================

#[derive(Accounts)]
#[instruction(round_id: u64)]
pub struct InitializeBettingRound<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + BettingRound::INIT_SPACE,
        seeds = [b"betting_round", round_id.to_le_bytes().as_ref()],
        bump
    )]
    pub betting_round: Account<'info, BettingRound>,

    #[account(
        mut,
        seeds = [b"escrow", round_id.to_le_bytes().as_ref()],
        bump
    )]
    pub escrow: SystemAccount<'info>,

    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: Treasury account for fee collection
    pub treasury: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct PlaceBet<'info> {
    #[account(mut)]
    pub betting_round: Account<'info, BettingRound>,

    #[account(
        init,
        payer = bettor,
        space = 8 + BetAccount::INIT_SPACE,
        seeds = [
            b"bet",
            betting_round.round_id.to_le_bytes().as_ref(),
            bettor.key().as_ref()
        ],
        bump
    )]
    pub bet_account: Account<'info, BetAccount>,

    #[account(
        mut,
        seeds = [b"escrow", betting_round.round_id.to_le_bytes().as_ref()],
        bump,
        constraint = escrow.key() != bettor.key() @ BettingError::InvalidAccount
    )]
    /// CHECK: This is safe because it's the escrow account
    pub escrow: UncheckedAccount<'info>,

    #[account(mut)]
    pub bettor: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct StartFightPhase<'info> {
    #[account(mut)]
    pub betting_round: Account<'info, BettingRound>,

    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct UpdateBossHp<'info> {
    #[account(mut)]
    pub betting_round: Account<'info, BettingRound>,

    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct EndFight<'info> {
    #[account(mut)]
    pub betting_round: Account<'info, BettingRound>,

    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct ClaimPayout<'info> {
    pub betting_round: Account<'info, BettingRound>,

    #[account(
        mut, 
        close = bettor, 
        constraint = bet_account.round_id == betting_round.round_id,
    )]
    pub bet_account: Account<'info, BetAccount>,

    #[account(
        mut,
        seeds = [b"escrow", betting_round.round_id.to_le_bytes().as_ref()],
        bump
    )]
    /// CHECK: This is safe because it's the escrow account
    pub escrow: UncheckedAccount<'info>,

    #[account(mut)]
    pub bettor: SystemAccount<'info>,

    #[account(
        constraint = authority.key() == betting_round.authority @ BettingError::Unauthorized
    )]
    pub authority: Signer<'info>, 

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ClaimFees<'info> {
    pub betting_round: Account<'info, BettingRound>,

    #[account(
        mut,
        seeds = [b"escrow", betting_round.round_id.to_le_bytes().as_ref()],
        bump
    )]
    /// CHECK: This is safe because it's the escrow account
    pub escrow: UncheckedAccount<'info>,

    #[account(mut)]
    /// CHECK: Treasury account for fee collection
    pub treasury: UncheckedAccount<'info>,

    pub authority: Signer<'info>, 

    pub system_program: Program<'info, System>,
}

// =================================================================
// ⭐️ ERROR CODES (MUST BE AT THE END) ⭐️
// =================================================================

#[error_code]
pub enum BettingError {
    #[msg("Not in betting phase")]
    NotInBettingPhase,
    #[msg("Betting period has expired")]
    BettingPeriodExpired,
    #[msg("Bet amount too small")]
    BetTooSmall,
    #[msg("Username too long")]
    UsernameTooLong,
    #[msg("Not in fight phase")]
    NotInFightPhase,
    #[msg("Betting is still active")]
    BettingStillActive,
    #[msg("Fight period has expired")]
    FightPeriodExpired,
    #[msg("Fight has not finished")]
    FightNotFinished,
    #[msg("Fight has not ended")]
    FightNotEnded,
    #[msg("Payout already claimed")]
    PayoutAlreadyClaimed,
    #[msg("Bet lost, no payout available")]
    BetLost,
    #[msg("Unauthorized")]
    Unauthorized,
    #[msg("Insufficient funds in escrow account for payout")]
    InsufficientEscrowFunds,
    #[msg("Invalid account provided")]
    InvalidAccount,
    #[msg("Arithmetic overflow")]
    ArithmeticOverflow,
}