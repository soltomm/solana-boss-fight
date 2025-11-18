use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer, Mint};
use anchor_spl::associated_token::AssociatedToken;

declare_id!("FtQbMDA7w8a9icfbMkuTxxQ695Wp9e6RQFSGVjmYQgz3");

// =================================================================
// ⭐️ DATA ACCOUNTS ⭐️
// =================================================================

#[account]
#[derive(InitSpace)]
pub struct BettingRound {
    pub round_id: u64,
    pub authority: Pubkey,
    pub treasury: Pubkey,
    pub token_mint: Pubkey,
    pub betting_start_time: i64,
    pub betting_end_time: i64,
    pub fight_end_time: i64,
    pub initial_hp: u32,
    pub current_hp: u32,
    pub prize_pool_amount: u64,  // CHANGED: Fixed prize pool from treasury
    pub phase: GamePhase,
    pub total_death_bets: u64,  // CHANGED: Now just counts number of death bets
    pub total_survival_bets: u64,  // CHANGED: Now just counts number of survival bets
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
// ✅ EVENTS ✅
// =================================================================

#[event]
pub struct BettingRoundInitialized {
    pub round_id: u64,
    pub betting_end_time: i64,
    pub fight_end_time: i64,
    pub token_mint: Pubkey,
    pub prize_pool_amount: u64,  // NEW
}

#[event]
pub struct BetPlaced {
    pub round_id: u64,
    pub bettor: Pubkey,
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
    pub payout_amount: u64,  // CHANGED: Just the equal share amount
}

// =================================================================
// ⭐️ PROGRAM INSTRUCTIONS ⭐️
// =================================================================
#[program]
pub mod boss_fight_betting {
    use super::*;

    /// Initialize a new betting round with treasury-funded prize pool
    pub fn initialize_betting_round(
        ctx: Context<InitializeBettingRound>,
        round_id: u64,
        betting_duration: i64,
        fight_duration: i64,
        initial_hp: u32,
        prize_pool_amount: u64,  // NEW: Treasury funds this amount
    ) -> Result<()> {
        let betting_round = &mut ctx.accounts.betting_round;
        let clock = Clock::get()?;

        betting_round.round_id = round_id;
        betting_round.authority = ctx.accounts.authority.key();
        betting_round.treasury = ctx.accounts.treasury.key();
        betting_round.token_mint = ctx.accounts.token_mint.key();
        betting_round.betting_start_time = clock.unix_timestamp;
        
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
        betting_round.prize_pool_amount = prize_pool_amount;
        betting_round.phase = GamePhase::Betting;
        betting_round.total_death_bets = 0;
        betting_round.total_survival_bets = 0;
        betting_round.total_bets_count = 0;
        betting_round.boss_defeated = false;
        betting_round.payouts_processed = false;
        betting_round.escrow_bump = ctx.bumps.escrow_token_account;

        // Transfer prize pool from treasury to escrow
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.treasury_token_account.to_account_info(),
                    to: ctx.accounts.escrow_token_account.to_account_info(),
                    authority: ctx.accounts.treasury.to_account_info(),
                },
            ),
            prize_pool_amount,
        )?;

        emit!(BettingRoundInitialized {
            round_id,
            betting_end_time: betting_round.betting_end_time,
            fight_end_time: betting_round.fight_end_time,
            token_mint: betting_round.token_mint,
            prize_pool_amount,
        });

        Ok(())
    }

    /// Place a bet on boss death or survival (NO TOKENS REQUIRED)
    pub fn place_bet(
        ctx: Context<PlaceBet>,
        prediction: BossPrediction,
        username: String,
    ) -> Result<()> {
        let betting_round = &mut ctx.accounts.betting_round;
        let bet_account = &mut ctx.accounts.bet_account;
        let clock = Clock::get()?;

        // Explicit signer check
        require!(
            ctx.accounts.bettor.is_signer,
            BettingError::Unauthorized
        );

        // Validate betting phase and timing
        require!(
            betting_round.phase == GamePhase::Betting,
            BettingError::NotInBettingPhase
        );
        require!(
            clock.unix_timestamp <= betting_round.betting_end_time,
            BettingError::BettingPeriodExpired
        );
        require!(username.len() <= 32, BettingError::UsernameTooLong);

        // Initialize bet account (NO TOKEN TRANSFER)
        bet_account.bettor = ctx.accounts.bettor.key();
        bet_account.round_id = betting_round.round_id;
        bet_account.prediction = prediction.clone();
        bet_account.username = username;
        bet_account.timestamp = clock.unix_timestamp;
        bet_account.payout_claimed = false;

        // Update betting round counts
        match prediction {
            BossPrediction::Death => betting_round.total_death_bets += 1,
            BossPrediction::Survival => betting_round.total_survival_bets += 1,
        }
        betting_round.total_bets_count += 1;

        emit!(BetPlaced {
            round_id: betting_round.round_id,
            bettor: ctx.accounts.bettor.key(),
            prediction: prediction.clone(),
            username: bet_account.username.clone(),
        });

        Ok(())
    }

    /// Start the fighting phase
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

    /// Update boss HP
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
    pub fn end_fight(ctx: Context<EndFight>, final_hp: u64) -> Result<()> {
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

        let fight_expired = clock.unix_timestamp >= betting_round.fight_end_time;
        
        betting_round.current_hp = final_hp as u32;
        let boss_dead = final_hp == 0;

        require!(fight_expired || boss_dead, BettingError::FightNotFinished);

        betting_round.phase = GamePhase::Ended;
        betting_round.boss_defeated = boss_dead;

        emit!(FightEnded {
            round_id: betting_round.round_id,
            boss_defeated: boss_dead,
        });

        Ok(())
    }

    /// Claim equal share of prize pool for winning bet
    pub fn claim_payout(ctx: Context<ClaimPayout>) -> Result<()> {
        let betting_round = &ctx.accounts.betting_round;
        let bet_account = &mut ctx.accounts.bet_account;

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

        // Calculate equal share
        let total_winners = if betting_round.boss_defeated {
            betting_round.total_death_bets
        } else {
            betting_round.total_survival_bets
        };

        require!(total_winners > 0, BettingError::NoWinners);

        // Equal split of prize pool
        let payout_amount = (betting_round.prize_pool_amount as u128)
            .checked_div(total_winners as u128)
            .ok_or(BettingError::ArithmeticOverflow)?;
        
        let payout_u64 = u64::try_from(payout_amount)
            .map_err(|_| BettingError::ArithmeticOverflow)?;
        
        require!(
            ctx.accounts.escrow_token_account.amount >= payout_u64,
            BettingError::InsufficientEscrowFunds
        );

        // Transfer equal share to winner
        let round_id_bytes = betting_round.round_id.to_le_bytes();
        let escrow_seeds: &[&[u8]] = &[
            b"escrow",
            round_id_bytes.as_ref(),
            &[betting_round.escrow_bump],
        ];
        let signer_seeds = &[&escrow_seeds[..]];

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.escrow_token_account.to_account_info(),
                    to: ctx.accounts.bettor_token_account.to_account_info(),
                    authority: ctx.accounts.escrow_token_account.to_account_info(),
                },
                signer_seeds
            ),
            payout_u64,
        )?;

        bet_account.payout_claimed = true;

        emit!(PayoutClaimed {
            round_id: betting_round.round_id,
            bettor: ctx.accounts.bettor.key(),
            payout_amount: payout_u64,
        });
        
        Ok(())
    }
}

// =================================================================
// ⭐️ ACCOUNTS CONTEXTS ⭐️
// =================================================================

#[derive(Accounts)]
#[instruction(round_id: u64)]
pub struct InitializeBettingRound<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + BettingRound::INIT_SPACE,
        seeds = [b"betting_round", round_id.to_le_bytes().as_ref()],
        bump,
        constraint = betting_round.key() != escrow_token_account.key() @ BettingError::InvalidAccount,
        constraint = betting_round.key() != token_mint.key() @ BettingError::InvalidAccount,
        constraint = betting_round.key() != treasury_token_account.key() @ BettingError::InvalidAccount,
        constraint = betting_round.key() != authority.key() @ BettingError::InvalidAccount,
        constraint = betting_round.key() != treasury.key() @ BettingError::InvalidAccount
    )]
    pub betting_round: Account<'info, BettingRound>,

    #[account(
        init,
        payer = authority,
        token::mint = token_mint,
        token::authority = escrow_token_account,
        seeds = [b"escrow", round_id.to_le_bytes().as_ref()],
        bump,
        constraint = escrow_token_account.key() != token_mint.key() @ BettingError::InvalidAccount,
        constraint = escrow_token_account.key() != treasury_token_account.key() @ BettingError::InvalidAccount,
        constraint = escrow_token_account.key() != authority.key() @ BettingError::InvalidAccount,
        constraint = escrow_token_account.key() != treasury.key() @ BettingError::InvalidAccount
    )]
    pub escrow_token_account: Account<'info, TokenAccount>,

    #[account(
        constraint = token_mint.key() != treasury_token_account.key() @ BettingError::InvalidAccount,
        constraint = token_mint.key() != authority.key() @ BettingError::InvalidAccount,
        constraint = token_mint.key() != treasury.key() @ BettingError::InvalidAccount,
        constraint = token_mint.key() != system_program.key() @ BettingError::InvalidAccount,
        constraint = token_mint.key() != token_program.key() @ BettingError::InvalidAccount,
        constraint = token_mint.key() != rent.key() @ BettingError::InvalidAccount
    )]
    pub token_mint: Account<'info, Mint>,

    // Treasury token account (must have funds to deposit prize pool)
    #[account(
        mut,
        constraint = treasury_token_account.mint == token_mint.key() @ BettingError::InvalidTokenMint,
        constraint = treasury_token_account.owner == treasury.key() @ BettingError::InvalidTokenAccount,
        constraint = treasury_token_account.key() != authority.key() @ BettingError::InvalidAccount,
        constraint = treasury_token_account.key() != treasury.key() @ BettingError::InvalidAccount,
        constraint = treasury_token_account.key() != system_program.key() @ BettingError::InvalidAccount,
        constraint = treasury_token_account.key() != token_program.key() @ BettingError::InvalidAccount
    )]
    pub treasury_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = authority.key() != system_program.key() @ BettingError::InvalidAccount,
        constraint = authority.key() != token_program.key() @ BettingError::InvalidAccount,
        constraint = authority.key() != rent.key() @ BettingError::InvalidAccount
    )]
    pub authority: Signer<'info>,

    /// CHECK: Treasury account (must sign to authorize prize pool deposit)
    #[account(
        mut,
        constraint = treasury.key() != system_program.key() @ BettingError::InvalidAccount,
        constraint = treasury.key() != token_program.key() @ BettingError::InvalidAccount,
        constraint = treasury.key() != rent.key() @ BettingError::InvalidAccount
    )]
    pub treasury: Signer<'info>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct PlaceBet<'info> {
    #[account(
        mut,
        constraint = betting_round.key() != bet_account.key() @ BettingError::InvalidAccount,
        constraint = betting_round.key() != bettor.key() @ BettingError::InvalidAccount,
        constraint = betting_round.key() != system_program.key() @ BettingError::InvalidAccount
    )]
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
        bump,
        constraint = bet_account.key() != bettor.key() @ BettingError::InvalidAccount,
        constraint = bet_account.key() != system_program.key() @ BettingError::InvalidAccount
    )]
    pub bet_account: Account<'info, BetAccount>,

    #[account(
        mut,
        constraint = bettor.key() != system_program.key() @ BettingError::InvalidAccount
    )]
    pub bettor: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct StartFightPhase<'info> {
    #[account(
        mut,
        constraint = betting_round.key() != authority.key() @ BettingError::InvalidAccount
    )]
    pub betting_round: Account<'info, BettingRound>,

    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct UpdateBossHp<'info> {
    #[account(
        mut,
        constraint = betting_round.key() != authority.key() @ BettingError::InvalidAccount
    )]
    pub betting_round: Account<'info, BettingRound>,

    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct EndFight<'info> {
    #[account(
        mut,
        constraint = betting_round.key() != authority.key() @ BettingError::InvalidAccount
    )]
    pub betting_round: Account<'info, BettingRound>,

    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct ClaimPayout<'info> {
    #[account(
        constraint = betting_round.key() != bet_account.key() @ BettingError::InvalidAccount,
        constraint = betting_round.key() != escrow_token_account.key() @ BettingError::InvalidAccount,
        constraint = betting_round.key() != bettor_token_account.key() @ BettingError::InvalidAccount,
        constraint = betting_round.key() != bettor.key() @ BettingError::InvalidAccount,
        constraint = betting_round.key() != token_program.key() @ BettingError::InvalidAccount
    )]
    pub betting_round: Account<'info, BettingRound>,

    #[account(
        mut, 
        close = bettor, 
        constraint = bet_account.round_id == betting_round.round_id,
        constraint = bet_account.key() != escrow_token_account.key() @ BettingError::InvalidAccount,
        constraint = bet_account.key() != bettor_token_account.key() @ BettingError::InvalidAccount,
        constraint = bet_account.key() != bettor.key() @ BettingError::InvalidAccount,
        constraint = bet_account.key() != token_program.key() @ BettingError::InvalidAccount
    )]
    pub bet_account: Account<'info, BetAccount>,

    #[account(
        mut,
        seeds = [b"escrow", betting_round.round_id.to_le_bytes().as_ref()],
        bump,
        constraint = escrow_token_account.mint == betting_round.token_mint @ BettingError::InvalidTokenMint,
        constraint = escrow_token_account.key() != bettor_token_account.key() @ BettingError::InvalidAccount,
        constraint = escrow_token_account.key() != bettor.key() @ BettingError::InvalidAccount,
        constraint = escrow_token_account.key() != token_program.key() @ BettingError::InvalidAccount
    )]
    pub escrow_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = bettor_token_account.owner == bettor.key() @ BettingError::InvalidTokenAccount,
        constraint = bettor_token_account.mint == betting_round.token_mint @ BettingError::InvalidTokenMint,
        constraint = bettor_token_account.key() != bettor.key() @ BettingError::InvalidAccount,
        constraint = bettor_token_account.key() != token_program.key() @ BettingError::InvalidAccount
    )]
    pub bettor_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = bettor.key() != token_program.key() @ BettingError::InvalidAccount
    )]
    pub bettor: SystemAccount<'info>,

    pub token_program: Program<'info, Token>,
}

// =================================================================
// ⭐️ ERROR CODES ⭐️
// =================================================================

#[error_code]
pub enum BettingError {
    #[msg("Not in betting phase")]
    NotInBettingPhase,
    #[msg("Betting period has expired")]
    BettingPeriodExpired,
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
    #[msg("Invalid token mint")]
    InvalidTokenMint,
    #[msg("Invalid token account")]
    InvalidTokenAccount,
    #[msg("No winners to distribute prize pool")]
    NoWinners,
}