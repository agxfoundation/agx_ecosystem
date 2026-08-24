use anchor_lang::prelude::*;
use anchor_spl::token::{self, Transfer, TokenAccount, Mint, Token};

declare_id!("niaMeiu7vpiCvRpTEtEcmexMGD4JP7yH8JeYkLyzsiz");

#[program]
pub mod agx_ecosystem {
    use super::*;

    /// Initialize the global state, vaults, and configuration variables.
    pub fn initialize(
        ctx: Context<Initialize>,
        sell_price: u64,
        swap_fee_percentage: u64,
        vault_authority_bump: u8,
    ) -> Result<()> {
        let state = &mut ctx.accounts.state;
        state.admin = *ctx.accounts.admin.key;
        state.pending_admin = Pubkey::default();
        state.token_mint = ctx.accounts.token_mint.key();
        state.usdt_mint = ctx.accounts.usdt_mint.key();
        state.usdt_vault = ctx.accounts.usdt_vault.key();
        state.reward_vault = ctx.accounts.reward_vault.key();
        state.presale_vault = ctx.accounts.presale_vault.key();
        
        state.treasury_vault = ctx.accounts.treasury_vault.key();
        state.development_vault = ctx.accounts.development_vault.key();
        state.marketing_vault = ctx.accounts.marketing_vault.key();
        state.roadmap_vault = ctx.accounts.roadmap_vault.key();

        state.vault_authority_bump = vault_authority_bump;

        state.buy_active = true;
        state.claim_active = true;
        state.stake_active = true;
        state.swap_active = true;
        state.emergency_paused = false;

        state.sell_price = sell_price; // Decimals matching USDT (6 decimals)
        state.swap_fee_percentage = swap_fee_percentage;
        
        state.tokens_sold_presale = 0;
        state.tokens_sold_reward = 0;
        state.transaction_count = 0;

        // Vault accounting initializations
        state.reward_vault_total = 25_000_000_000_000_000u64; // 25M with 9 decimals
        state.reward_vault_reserved = 0;
        state.reward_vault_available = 25_000_000_000_000_000u64;
        state.reward_paid = 0;
        state.reward_returned = 0;
        state.reward_sold = 0;

        state.sale_completed = false;
        state.reward_sale_completed = false;
        
        state.reward_counter = 0;
        state.returned_counter = 0;
        state.sale_counter = 0; // Cumulative AGX tokens sold
        state.sale_transactions = 0; // Cumulative transaction count

        state.treasury_claimed = 0;
        state.development_claimed = 0;
        state.marketing_claimed = 0;
        state.roadmap_claimed = 0;

        // Initialize actual block timestamps for operational vaults release countdowns
        let now = Clock::get()?.unix_timestamp;
        state.treasury_start_time = now;
        state.development_start_time = now;
        state.marketing_start_time = now;
        state.roadmap_start_time = now;

        emit!(ProgramEvent {
            event_type: 7, // Config Update
            user: state.admin,
            amount_1: sell_price,
            amount_2: swap_fee_percentage,
            record_id: 0,
        });

        Ok(())
    }

    /// Admin updates global toggles, pricing rules, and swap fees.
    pub fn update_config(
        ctx: Context<UpdateConfig>,
        buy_active: bool,
        claim_active: bool,
        stake_active: bool,
        swap_active: bool,
        sell_price: u64,
        swap_fee_percentage: u64,
    ) -> Result<()> {
        let state = &mut ctx.accounts.state;
        require!(!state.emergency_paused, AGXError::EmergencyPaused);

        state.buy_active = buy_active;
        state.claim_active = claim_active;
        state.stake_active = stake_active;
        state.swap_active = swap_active;
        state.sell_price = sell_price;
        state.swap_fee_percentage = swap_fee_percentage;

        emit!(ProgramEvent {
            event_type: 7, // Config Update
            user: state.admin,
            amount_1: sell_price,
            amount_2: swap_fee_percentage,
            record_id: 0,
        });

        Ok(())
    }

    /// Buy and Stake instantly in one transaction (clean frontend flow).
    pub fn buy_and_stake(
        ctx: Context<BuyAndStake>,
        usdt_amount: u64,
    ) -> Result<()> {
        let state = &mut ctx.accounts.state;
        require!(!state.emergency_paused, AGXError::EmergencyPaused);
        require!(state.buy_active, AGXError::PurchaseInactive);
        require!(state.stake_active, AGXError::StakingInactive);
        require!(!state.sale_completed, AGXError::SalesCompleted);
        require!(!state.reward_sale_completed, AGXError::RewardSaleCompleted);
        require!(usdt_amount >= 100_000_000u64, AGXError::BelowMinStake);

        // 1. Calculate active price and equivalent AGX
        let current_price = get_active_price(state)?;
        let agx_amount = usdt_amount.checked_mul(1_000_000_000u64).unwrap().checked_div(current_price).unwrap();

        // 2. Verification of total hard cap constraint BEFORE updating counters
        let total_sold = state.tokens_sold_presale.checked_add(state.tokens_sold_reward).unwrap();
        let new_total_sold = total_sold.checked_add(agx_amount).unwrap();
        require!(new_total_sold <= 30_000_000_000_000_000u64, AGXError::HardCapReached);

        // 3. Update tokens sold and reward pool allocations safely
        if state.tokens_sold_presale < 5_000_000_000_000_000u64 {
            let presale_rem = 5_000_000_000_000_000u64.checked_sub(state.tokens_sold_presale).unwrap();
            if agx_amount > presale_rem {
                state.tokens_sold_presale = 5_000_000_000_000_000u64;
                let reward_part = agx_amount.checked_sub(presale_rem).unwrap();
                state.tokens_sold_reward = state.tokens_sold_reward.checked_add(reward_part).unwrap();
                state.reward_sold = state.reward_sold.checked_add(reward_part).unwrap();
            } else {
                state.tokens_sold_presale = state.tokens_sold_presale.checked_add(agx_amount).unwrap();
            }
        } else {
            state.tokens_sold_reward = state.tokens_sold_reward.checked_add(agx_amount).unwrap();
            state.reward_sold = state.reward_sold.checked_add(agx_amount).unwrap();
        }

        // 4. Update completion flags if limits are met
        if state.tokens_sold_reward >= 25_000_000_000_000_000u64 {
            state.reward_sale_completed = true;
        }
        if state.tokens_sold_presale.checked_add(state.tokens_sold_reward).unwrap() >= 30_000_000_000_000_000u64 {
            state.sale_completed = true;
        }

        // Determine Staking Tier details based on USDT amount
        let (duration_months, multiplier) = determine_staking_tier(usdt_amount)?;
        let total_reward = agx_amount.checked_mul(multiplier as u64).unwrap();

        // Update Reward Vault allocations
        require!(state.reward_vault_available >= total_reward, AGXError::InsufficientRewardVault);
        state.reward_vault_reserved = state.reward_vault_reserved.checked_add(total_reward).unwrap();
        state.reward_vault_available = state.reward_vault_total.checked_sub(state.reward_vault_reserved).unwrap();

        // 5. Transfer USDT from User to USDT Vault
        let cpi_accounts = Transfer {
            from: ctx.accounts.user_usdt.to_account_info(),
            to: ctx.accounts.usdt_vault.to_account_info(),
            authority: ctx.accounts.user.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        token::transfer(cpi_ctx, usdt_amount)?;

        // 6. Save Staking Record inside the user's data account
        let stake_record = &mut ctx.accounts.stake_record;
        stake_record.record_id = state.transaction_count;
        stake_record.user = *ctx.accounts.user.key;
        stake_record.staked_amount_usdt = usdt_amount;
        stake_record.equivalent_agx = agx_amount;
        stake_record.total_reward_tokens = total_reward;
        stake_record.released_tokens = 0;
        stake_record.lock_duration_months = duration_months;
        let now = Clock::get()?.unix_timestamp;
        stake_record.purchase_time = now;
        stake_record.last_claim_time = now;
        stake_record.is_refunded = false;
        stake_record.is_staked = true; 

        // Increment counts
        state.transaction_count = state.transaction_count.checked_add(1).unwrap();
        state.sale_counter = state.sale_counter.checked_add(agx_amount).unwrap();
        state.sale_transactions = state.sale_transactions.checked_add(1).unwrap();
        state.reward_counter = state.reward_counter.checked_add(1).unwrap();

        emit!(ProgramEvent {
            event_type: 1, // Buy
            user: *ctx.accounts.user.key,
            amount_1: usdt_amount,
            amount_2: agx_amount,
            record_id: stake_record.record_id,
        });

        Ok(())
    }

    /// Claim unlocked rewards linearly on a monthly/epoch schedule.
    pub fn claim_rewards(
        ctx: Context<ClaimRewards>,
        record_id: u64,
    ) -> Result<()> {
        let state = &mut ctx.accounts.state;
        require!(!state.emergency_paused, AGXError::EmergencyPaused);
        require!(state.claim_active, AGXError::ClaimsInactive);

        let stake_record = &mut ctx.accounts.stake_record;
        require!(stake_record.is_staked, AGXError::NotStaked);
        require!(!stake_record.is_refunded, AGXError::AlreadyRefunded);
        require!(stake_record.record_id == record_id, AGXError::InvalidRecordId);

        let now = Clock::get()?.unix_timestamp;
        let elapsed = now.checked_sub(stake_record.purchase_time).unwrap();
        let total_lock_time = (stake_record.lock_duration_months as i64)
            .checked_mul(30 * 24 * 60 * 60).unwrap(); // 30 days per month

        require!(elapsed > 0, AGXError::NoClaimableRewards);

        let total_claimable = if elapsed >= total_lock_time {
            stake_record.total_reward_tokens
        } else {
            let elapsed_bn = elapsed as u128;
            let total_lock_bn = total_lock_time as u128;
            let total_reward_bn = stake_record.total_reward_tokens as u128;
            total_reward_bn.checked_mul(elapsed_bn).unwrap().checked_div(total_lock_bn).unwrap() as u64
        };

        let pending_to_claim = total_claimable.checked_sub(stake_record.released_tokens).unwrap();
        require!(pending_to_claim > 0, AGXError::NoClaimableRewards);

        stake_record.released_tokens = stake_record.released_tokens.checked_add(pending_to_claim).unwrap();
        stake_record.last_claim_time = now;

        state.reward_vault_reserved = state.reward_vault_reserved.checked_sub(pending_to_claim).unwrap();
        state.reward_paid = state.reward_paid.checked_add(pending_to_claim).unwrap();

        // Transfer tokens from Reward Vault PDA (using program authority) to User Token Account
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_accounts = Transfer {
            from: ctx.accounts.reward_vault.to_account_info(),
            to: ctx.accounts.user_token.to_account_info(),
            authority: ctx.accounts.vault_authority.to_account_info(),
        };

        let signer_seeds: &[&[&[u8]]] = &[&[
            b"vault-authority",
            &[state.vault_authority_bump],
        ]];

        let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer_seeds);
        token::transfer(cpi_ctx, pending_to_claim)?;

        emit!(ProgramEvent {
            event_type: 2, // Claim
            user: stake_record.user,
            amount_1: pending_to_claim,
            amount_2: total_claimable,
            record_id: stake_record.record_id,
        });

        Ok(())
    }

    /// Claim 100% refund of USDT if requested within 7 days of buy_and_stake.
    pub fn claim_refund(
        ctx: Context<ClaimRefund>,
        record_id: u64,
    ) -> Result<()> {
        let state = &mut ctx.accounts.state;
        require!(!state.emergency_paused, AGXError::EmergencyPaused);

        let stake_record = &mut ctx.accounts.stake_record;
        require!(stake_record.is_staked, AGXError::NotStaked);
        require!(!stake_record.is_refunded, AGXError::AlreadyRefunded);
        require!(stake_record.released_tokens == 0, AGXError::RefundForbiddenClaimed);
        require!(stake_record.record_id == record_id, AGXError::InvalidRecordId);

        // Refund Window verification (Strict 7 days = 604,800 seconds)
        let now = Clock::get()?.unix_timestamp;
        let elapsed = now.checked_sub(stake_record.purchase_time).unwrap();
        require!(elapsed <= 7 * 24 * 60 * 60, AGXError::RefundWindowExpired);

        stake_record.is_refunded = true;
        stake_record.is_staked = false;

        // Clean accounting stats
        let total_reward = stake_record.total_reward_tokens;
        state.reward_vault_reserved = state.reward_vault_reserved.checked_sub(total_reward).unwrap();
        state.reward_vault_available = state.reward_vault_total.checked_sub(state.reward_vault_reserved).unwrap();

        state.tokens_sold_reward = state.tokens_sold_reward.saturating_sub(stake_record.equivalent_agx);
        state.reward_sold = state.reward_sold.saturating_sub(stake_record.equivalent_agx);

        state.returned_counter = state.returned_counter.checked_add(1).unwrap();
        state.reward_returned = state.reward_returned.checked_add(stake_record.equivalent_agx).unwrap();

        // Transfer USDT back from USDT Vault (using program authority) to User USDT Account
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_accounts = Transfer {
            from: ctx.accounts.usdt_vault.to_account_info(),
            to: ctx.accounts.user_usdt.to_account_info(),
            authority: ctx.accounts.vault_authority.to_account_info(),
        };

        let signer_seeds: &[&[&[u8]]] = &[&[
            b"vault-authority",
            &[state.vault_authority_bump],
        ]];

        let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer_seeds);
        token::transfer(cpi_ctx, stake_record.staked_amount_usdt)?;

        emit!(ProgramEvent {
            event_type: 3, // Refund
            user: stake_record.user,
            amount_1: stake_record.staked_amount_usdt,
            amount_2: stake_record.equivalent_agx,
            record_id: stake_record.record_id,
        });

        Ok(())
    }

    /// Swap USDT directly for AGX (instantly transferred to user, no lock).
    pub fn swap_t20(
        ctx: Context<SwapT20>,
        usdt_amount: u64,
    ) -> Result<()> {
        let state = &mut ctx.accounts.state;
        require!(!state.emergency_paused, AGXError::EmergencyPaused);
        require!(state.swap_active, AGXError::SwapInactive);
        require!(!state.sale_completed, AGXError::SalesCompleted);

        // 1. Calculate tokens based on admin-defined swap price
        let current_price = state.sell_price;
        require!(current_price > 0, AGXError::PurchaseInactive);
        let agx_amount = usdt_amount.checked_mul(1_000_000_000u64).unwrap().checked_div(current_price).unwrap();

        // 2. Transfer USDT from User to USDT Vault
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_accounts = Transfer {
            from: ctx.accounts.user_usdt.to_account_info(),
            to: ctx.accounts.usdt_vault.to_account_info(),
            authority: ctx.accounts.user.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(cpi_program.clone(), cpi_accounts);
        token::transfer(cpi_ctx, usdt_amount)?;

        // 3. Transfer AGX from Presale Vault (using program authority) to User Token Account
        let cpi_accounts_agx = Transfer {
            from: ctx.accounts.presale_vault.to_account_info(),
            to: ctx.accounts.user_token.to_account_info(),
            authority: ctx.accounts.vault_authority.to_account_info(),
        };

        let signer_seeds: &[&[&[u8]]] = &[&[
            b"vault-authority",
            &[state.vault_authority_bump],
        ]];

        let cpi_ctx_agx = CpiContext::new_with_signer(cpi_program, cpi_accounts_agx, signer_seeds);
        token::transfer(cpi_ctx_agx, agx_amount)?;

        // Cumulative accounting logs (without modifying price-determining counters)
        state.transaction_count = state.transaction_count.checked_add(1).unwrap();
        state.sale_counter = state.sale_counter.checked_add(agx_amount).unwrap();
        state.sale_transactions = state.sale_transactions.checked_add(1).unwrap();

        emit!(ProgramEvent {
            event_type: 4, // Swap
            user: *ctx.accounts.user.key,
            amount_1: usdt_amount,
            amount_2: agx_amount,
            record_id: 0,
        });

        Ok(())
    }

    /// Swap AGX back for USDT (direct selling back to contract with an admin fee).
    pub fn swap_agx_to_usdt(
        ctx: Context<SwapAgxToUsdt>,
        agx_amount: u64,
    ) -> Result<()> {
        let state = &mut ctx.accounts.state;
        require!(!state.emergency_paused, AGXError::EmergencyPaused);
        require!(state.swap_active, AGXError::SwapInactive);

        let current_price = state.sell_price;
        require!(current_price > 0, AGXError::PurchaseInactive);

        // 1. Calculate Admin Swap Fee: agx_amount * fee_pct / 100
        let fee_amount = agx_amount.checked_mul(state.swap_fee_percentage).unwrap().checked_div(100).unwrap();
        let net_agx = agx_amount.checked_sub(fee_amount).unwrap();

        // 2. Calculate USDT output: (net_agx * price) / 10^9
        let usdt_amount = net_agx.checked_mul(current_price).unwrap().checked_div(1_000_000_000u64).unwrap();
        require!(usdt_amount > 0, AGXError::BelowMinStake);

        // 3. User transfers full AGX (net + fee) to the Presale Vault
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_accounts_agx = Transfer {
            from: ctx.accounts.user_token.to_account_info(),
            to: ctx.accounts.presale_vault.to_account_info(),
            authority: ctx.accounts.user.to_account_info(),
        };
        let cpi_ctx_agx = CpiContext::new(cpi_program.clone(), cpi_accounts_agx);
        token::transfer(cpi_ctx_agx, agx_amount)?;

        // 4. Contract transfers USDT from USDT Vault to the User
        let cpi_accounts_usdt = Transfer {
            from: ctx.accounts.usdt_vault.to_account_info(),
            to: ctx.accounts.user_usdt.to_account_info(),
            authority: ctx.accounts.vault_authority.to_account_info(),
        };

        let signer_seeds: &[&[&[u8]]] = &[&[
            b"vault-authority",
            &[state.vault_authority_bump],
        ]];

        let cpi_ctx_usdt = CpiContext::new_with_signer(cpi_program, cpi_accounts_usdt, signer_seeds);
        token::transfer(cpi_ctx_usdt, usdt_amount)?;

        emit!(ProgramEvent {
            event_type: 4, // Swap (Selling back to contract)
            user: *ctx.accounts.user.key,
            amount_1: agx_amount,
            amount_2: usdt_amount,
            record_id: 1, // 1 represents swap-back
        });

        Ok(())
    }

    /// Claim monthly operational team allocations (Treasury, Development, Marketing, Roadmap).
    pub fn claim_operational_vault(
        ctx: Context<ClaimOperationalVault>,
        vault_type: u8,
    ) -> Result<()> {
        let state = &mut ctx.accounts.state;
        require!(!state.emergency_paused, AGXError::EmergencyPaused);

        let now = Clock::get()?.unix_timestamp;
        let mut transfer_amount: u64 = 0;

        match vault_type {
            1 => { // Treasury Vault: 5M tokens total, 12 months lock, 1% monthly release
                let elapsed = now.checked_sub(state.treasury_start_time).unwrap();
                let lock_period = 12 * 30 * 24 * 60 * 60; // 12 Months Lock
                require!(elapsed >= lock_period, AGXError::OperationalTimeLocked);
                
                let months_since_lock = elapsed.checked_sub(lock_period).unwrap().checked_div(30 * 24 * 60 * 60).unwrap() as u64;
                let claimable_total = (months_since_lock.checked_add(1).unwrap())
                    .checked_mul(50_000_000_000_000u64).unwrap(); // 1% = 50,000 tokens (9 decimals)
                let to_claim = claimable_total.saturating_sub(state.treasury_claimed);
                require!(to_claim > 0, AGXError::OperationalTimeLocked);
                require!(state.treasury_claimed.checked_add(to_claim).unwrap() <= 5_000_000_000_000_000u64, AGXError::MaxOperationalLimitReached);
                
                state.treasury_claimed = state.treasury_claimed.checked_add(to_claim).unwrap();
                transfer_amount = to_claim;
            },
            2 => { // Development Vault: 5M tokens total, 8 months lock, 0.5% monthly release
                let elapsed = now.checked_sub(state.development_start_time).unwrap();
                let lock_period = 8 * 30 * 24 * 60 * 60; // 8 Months Lock
                require!(elapsed >= lock_period, AGXError::OperationalTimeLocked);
                
                let months_since_lock = elapsed.checked_sub(lock_period).unwrap().checked_div(30 * 24 * 60 * 60).unwrap() as u64;
                let claimable_total = (months_since_lock.checked_add(1).unwrap())
                    .checked_mul(25_000_000_000_000u64).unwrap(); // 0.5% = 25,000 tokens (9 decimals)
                let to_claim = claimable_total.saturating_sub(state.development_claimed);
                require!(to_claim > 0, AGXError::OperationalTimeLocked);
                require!(state.development_claimed.checked_add(to_claim).unwrap() <= 5_000_000_000_000_000u64, AGXError::MaxOperationalLimitReached);

                state.development_claimed = state.development_claimed.checked_add(to_claim).unwrap();
                transfer_amount = to_claim;
            },
            3 => { // Marketing Vault: 5M tokens total, 6 months lock, 0.25% monthly release
                let elapsed = now.checked_sub(state.marketing_start_time).unwrap();
                let lock_period = 6 * 30 * 24 * 60 * 60; // 6 Months Lock
                require!(elapsed >= lock_period, AGXError::OperationalTimeLocked);
                
                let months_since_lock = elapsed.checked_sub(lock_period).unwrap().checked_div(30 * 24 * 60 * 60).unwrap() as u64;
                let claimable_total = (months_since_lock.checked_add(1).unwrap())
                    .checked_mul(12_500_000_000_000u64).unwrap(); // 0.25% = 12,500 tokens (9 decimals)
                let to_claim = claimable_total.saturating_sub(state.marketing_claimed);
                require!(to_claim > 0, AGXError::OperationalTimeLocked);
                require!(state.marketing_claimed.checked_add(to_claim).unwrap() <= 5_000_000_000_000_000u64, AGXError::MaxOperationalLimitReached);

                state.marketing_claimed = state.marketing_claimed.checked_add(to_claim).unwrap();
                transfer_amount = to_claim;
            },
            4 => { // Roadmap Vault: 5M tokens total, 12 months lock, 0.10% monthly release
                let elapsed = now.checked_sub(state.roadmap_start_time).unwrap();
                let lock_period = 12 * 30 * 24 * 60 * 60; // 12 Months Lock
                require!(elapsed >= lock_period, AGXError::OperationalTimeLocked);
                
                let months_since_lock = elapsed.checked_sub(lock_period).unwrap().checked_div(30 * 24 * 60 * 60).unwrap() as u64;
                let claimable_total = (months_since_lock.checked_add(1).unwrap())
                    .checked_mul(5_000_000_000_000u64).unwrap(); // 0.10% = 5,000 tokens (9 decimals)
                let to_claim = claimable_total.saturating_sub(state.roadmap_claimed);
                require!(to_claim > 0, AGXError::OperationalTimeLocked);
                require!(state.roadmap_claimed.checked_add(to_claim).unwrap() <= 5_000_000_000_000_000u64, AGXError::MaxOperationalLimitReached);

                state.roadmap_claimed = state.roadmap_claimed.checked_add(to_claim).unwrap();
                transfer_amount = to_claim;
            },
            _ => return Err(AGXError::InvalidVaultType.into()),
        }

        require!(transfer_amount > 0, AGXError::NoClaimableRewards);

        // Perform CPI transfer depending on type
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let target_vault = match vault_type {
            1 => ctx.accounts.treasury_vault.to_account_info(),
            2 => ctx.accounts.development_vault.to_account_info(),
            3 => ctx.accounts.marketing_vault.to_account_info(),
            4 => ctx.accounts.roadmap_vault.to_account_info(),
            _ => unreachable!(),
        };

        let signer_seeds: &[&[&[u8]]] = &[&[
            b"vault-authority",
            &[state.vault_authority_bump],
        ]];

        let cpi_accounts = Transfer {
            from: target_vault,
            to: ctx.accounts.user_token.to_account_info(),
            authority: ctx.accounts.vault_authority.to_account_info(),
        };

        let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer_seeds);
        token::transfer(cpi_ctx, transfer_amount)?;

        emit!(ProgramEvent {
            event_type: 5, // Operational Claim
            user: *ctx.accounts.admin.key,
            amount_1: transfer_amount,
            amount_2: vault_type as u64,
            record_id: 0,
        });

        Ok(())
    }

    /// Admin initiates the transfer of administrative control.
    pub fn transfer_admin(
        ctx: Context<TransferAdmin>,
        new_admin: Pubkey,
    ) -> Result<()> {
        let state = &mut ctx.accounts.state;
        require!(!state.emergency_paused, AGXError::EmergencyPaused);
        require!(new_admin != Pubkey::default(), AGXError::InvalidAdminAddress);

        state.pending_admin = new_admin;

        emit!(ProgramEvent {
            event_type: 8, // Admin Transfer Initiated
            user: *ctx.accounts.admin.key,
            amount_1: 0,
            amount_2: 0,
            record_id: 0,
        });

        Ok(())
    }

    /// Pending admin accepts the transfer of control.
    pub fn accept_admin(ctx: Context<AcceptAdmin>) -> Result<()> {
        let state = &mut ctx.accounts.state;
        require!(!state.emergency_paused, AGXError::EmergencyPaused);
        require!(*ctx.accounts.pending_admin.key == state.pending_admin, AGXError::Unauthorized);

        state.admin = state.pending_admin;
        state.pending_admin = Pubkey::default();

        emit!(ProgramEvent {
            event_type: 9, // Admin Transfer Accepted
            user: state.admin,
            amount_1: 0,
            amount_2: 0,
            record_id: 0,
        });

        Ok(())
    }

    /// Emergency switch to lock the contract instantly.
    pub fn set_emergency_pause(
        ctx: Context<SetEmergencyPause>,
        paused: bool,
    ) -> Result<()> {
        let state = &mut ctx.accounts.state;
        state.emergency_paused = paused;

        emit!(ProgramEvent {
            event_type: 6, // Emergency Pause
            user: *ctx.accounts.admin.key,
            amount_1: if paused { 1 } else { 0 },
            amount_2: 0,
            record_id: 0,
        });

        Ok(())
    }
}

fn get_active_price(state: &GlobalState) -> Result<u64> {
    let total_sold = state.tokens_sold_presale.checked_add(state.tokens_sold_reward).unwrap();
    
    if total_sold < 1_000_000_000_000_000u64 { // Less than 1M tokens (9 decimals)
        Ok(62_000u64) // 0.062 USDT (6 decimals)
    } else if total_sold < 2_000_000_000_000_000u64 { // 1M to 2M tokens
        Ok(72_000u64) // 0.072 USDT
    } else if total_sold < 3_000_000_000_000_000u64 { // 2M to 3M tokens
        Ok(85_000u64) // 0.085 USDT
    } else if total_sold < 4_000_000_000_000_000u64 { // 3M to 4M tokens
        Ok(95_000u64) // 0.095 USDT
    } else if total_sold < 5_000_000_000_000_000u64 { // 4M to 5M tokens
        Ok(100_000u64) // 0.10 USDT
    } else {
        // Phase 2: After 5M tokens sold
        // Base is 0.10 USDT (100,000 units), rises by 0.015% of base ($0.000015 = 15 units)
        // per 1,000 tokens sold (1_000_000_000_000 units in 9 decimals)
        let phase2_sold = total_sold.checked_sub(5_000_000_000_000_000u64).unwrap();
        let thousand_tokens_chunks = phase2_sold.checked_div(1_000_000_000_000u64).unwrap();
        let increment_amount = thousand_tokens_chunks.checked_mul(15u64).unwrap();
        let phase2_base_price = 100_000u64; // 0.10 USDT
        Ok(phase2_base_price.checked_add(increment_amount).unwrap())
    }
}

fn determine_staking_tier(usdt_amount: u64) -> Result<(u8, u8)> {
    if usdt_amount >= 100_000_000u64 && usdt_amount <= 2_500_000_000u64 {
        Ok((20, 2)) // $100 to $2,500: 20 Months Lock, 2X multiplier
    } else if usdt_amount > 2_500_000_000u64 && usdt_amount <= 5_000_000_000u64 {
        Ok((30, 3)) // $2,500.01 to $5,000: 30 Months Lock, 3X multiplier
    } else if usdt_amount > 5_000_000_000u64 {
        Ok((40, 4)) // Above $5,000: 40 Months Lock, 4X multiplier
    } else {
        Err(AGXError::BelowMinStake.into())
    }
}

// Data Account Validation Structures (Without dynamic PDA init blocks to keep tx footprint small)
#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = admin,
        space = 8 + 700, // Safe headroom allocation for global states (supports structural expansions)
    )]
    pub state: Account<'info, GlobalState>,

    pub token_mint: Box<Account<'info, Mint>>,
    pub usdt_mint: Box<Account<'info, Mint>>,

    /// USDT Vault (Pre-created with vault_authority as owner)
    #[account(mut)]
    pub usdt_vault: Box<Account<'info, TokenAccount>>,

    /// Reward Pool Vault (Pre-created with vault_authority as owner)
    #[account(mut)]
    pub reward_vault: Box<Account<'info, TokenAccount>>,

    /// Presale Vault (Pre-created with vault_authority as owner)
    #[account(mut)]
    pub presale_vault: Box<Account<'info, TokenAccount>>,

    /// Operational Vault: Treasury (Pre-created with vault_authority as owner)
    #[account(mut)]
    pub treasury_vault: Box<Account<'info, TokenAccount>>,

    /// Operational Vault: Development (Pre-created with vault_authority as owner)
    #[account(mut)]
    pub development_vault: Box<Account<'info, TokenAccount>>,

    /// Operational Vault: Marketing (Pre-created with vault_authority as owner)
    #[account(mut)]
    pub marketing_vault: Box<Account<'info, TokenAccount>>,

    /// Operational Vault: Roadmap (Pre-created with vault_authority as owner)
    #[account(mut)]
    pub roadmap_vault: Box<Account<'info, TokenAccount>>,

    /// Safe check verification that the vault_authority matches the program derived address.
    /// CHECK: Safe checking PDA derivation seed validation
    #[account(
        seeds = [b"vault-authority"],
        bump,
    )]
    pub vault_authority: UncheckedAccount<'info>,

    #[account(mut)]
    pub admin: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct UpdateConfig<'info> {
    #[account(mut, has_one = admin)]
    pub state: Account<'info, GlobalState>,
    pub admin: Signer<'info>,
}

#[derive(Accounts)]
pub struct BuyAndStake<'info> {
    #[account(mut)]
    pub state: Account<'info, GlobalState>,

    /// User's Staking Record data account
    #[account(
        init,
        payer = user,
        space = 8 + 128,
        seeds = [b"stake-record", user.key().as_ref(), &state.transaction_count.to_le_bytes()],
        bump,
    )]
    pub stake_record: Account<'info, StakingRecord>,

    /// USDT Mint
    pub usdt_mint: Box<Account<'info, Mint>>,

    /// USDT Vault
    #[account(mut, constraint = usdt_vault.key() == state.usdt_vault)]
    pub usdt_vault: Box<Account<'info, TokenAccount>>,

    /// User's USDT token wallet
    #[account(mut)]
    pub user_usdt: Box<Account<'info, TokenAccount>>,

    #[account(mut)]
    pub user: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ClaimRewards<'info> {
    #[account(mut)]
    pub state: Box<Account<'info, GlobalState>>,

    /// User's Staking Record
    #[account(
        mut,
        has_one = user
    )]
    pub stake_record: Account<'info, StakingRecord>,

    /// AGX Mint
    pub token_mint: Box<Account<'info, Mint>>,

    /// Reward Vault
    #[account(mut, constraint = reward_vault.key() == state.reward_vault)]
    pub reward_vault: Box<Account<'info, TokenAccount>>,

    /// User's AGX Token Account
    #[account(mut)]
    pub user_token: Box<Account<'info, TokenAccount>>,

    /// CHECK: Vault Authority PDA
    #[account(
        seeds = [b"vault-authority"],
        bump = state.vault_authority_bump,
    )]
    pub vault_authority: UncheckedAccount<'info>,

    #[account(mut)]
    pub user: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ClaimRefund<'info> {
    #[account(mut)]
    pub state: Box<Account<'info, GlobalState>>,

    /// User's Staking Record
    #[account(
        mut,
        has_one = user
    )]
    pub stake_record: Account<'info, StakingRecord>,

    /// USDT Mint
    pub usdt_mint: Box<Account<'info, Mint>>,

    /// USDT Vault
    #[account(mut, constraint = usdt_vault.key() == state.usdt_vault)]
    pub usdt_vault: Box<Account<'info, TokenAccount>>,

    /// User's USDT Token Account
    #[account(mut)]
    pub user_usdt: Box<Account<'info, TokenAccount>>,

    /// CHECK: Vault Authority PDA
    #[account(
        seeds = [b"vault-authority"],
        bump = state.vault_authority_bump,
    )]
    pub vault_authority: UncheckedAccount<'info>,

    #[account(mut)]
    pub user: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SwapT20<'info> {
    #[account(mut)]
    pub state: Box<Account<'info, GlobalState>>,

    /// USDT Mint
    pub usdt_mint: Box<Account<'info, Mint>>,

    /// AGX Mint
    pub token_mint: Box<Account<'info, Mint>>,

    /// USDT Vault
    #[account(mut, constraint = usdt_vault.key() == state.usdt_vault)]
    pub usdt_vault: Box<Account<'info, TokenAccount>>,

    /// Presale Vault
    #[account(mut, constraint = presale_vault.key() == state.presale_vault)]
    pub presale_vault: Box<Account<'info, TokenAccount>>,

    /// User's USDT Token Account
    #[account(mut)]
    pub user_usdt: Box<Account<'info, TokenAccount>>,

    /// User's AGX Token Account
    #[account(mut)]
    pub user_token: Box<Account<'info, TokenAccount>>,

    /// CHECK: Vault Authority PDA
    #[account(
        seeds = [b"vault-authority"],
        bump = state.vault_authority_bump,
    )]
    pub vault_authority: UncheckedAccount<'info>,

    #[account(mut)]
    pub user: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SwapAgxToUsdt<'info> {
    #[account(mut)]
    pub state: Box<Account<'info, GlobalState>>,

    /// USDT Mint
    pub usdt_mint: Box<Account<'info, Mint>>,

    /// AGX Mint
    pub token_mint: Box<Account<'info, Mint>>,

    /// USDT Vault
    #[account(mut, constraint = usdt_vault.key() == state.usdt_vault)]
    pub usdt_vault: Box<Account<'info, TokenAccount>>,

    /// Presale Vault
    #[account(mut, constraint = presale_vault.key() == state.presale_vault)]
    pub presale_vault: Box<Account<'info, TokenAccount>>,

    /// User's USDT Token Account
    #[account(mut)]
    pub user_usdt: Box<Account<'info, TokenAccount>>,

    /// User's AGX Token Account
    #[account(mut)]
    pub user_token: Box<Account<'info, TokenAccount>>,

    /// CHECK: Vault Authority PDA
    #[account(
        seeds = [b"vault-authority"],
        bump = state.vault_authority_bump,
    )]
    pub vault_authority: UncheckedAccount<'info>,

    #[account(mut)]
    pub user: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ClaimOperationalVault<'info> {
    #[account(mut, has_one = admin)]
    pub state: Box<Account<'info, GlobalState>>,

    /// Operational Vault: Treasury
    #[account(mut, constraint = treasury_vault.key() == state.treasury_vault)]
    pub treasury_vault: Box<Account<'info, TokenAccount>>,

    /// Operational Vault: Development
    #[account(mut, constraint = development_vault.key() == state.development_vault)]
    pub development_vault: Box<Account<'info, TokenAccount>>,

    /// Operational Vault: Marketing
    #[account(mut, constraint = marketing_vault.key() == state.marketing_vault)]
    pub marketing_vault: Box<Account<'info, TokenAccount>>,

    /// Operational Vault: Roadmap
    #[account(mut, constraint = roadmap_vault.key() == state.roadmap_vault)]
    pub roadmap_vault: Box<Account<'info, TokenAccount>>,

    /// User's AGX Token Account (destination)
    #[account(mut)]
    pub user_token: Box<Account<'info, TokenAccount>>,

    /// CHECK: Vault Authority PDA
    #[account(
        seeds = [b"vault-authority"],
        bump = state.vault_authority_bump,
    )]
    pub vault_authority: UncheckedAccount<'info>,

    pub admin: Signer<'info>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TransferAdmin<'info> {
    #[account(mut, has_one = admin)]
    pub state: Account<'info, GlobalState>,
    pub admin: Signer<'info>,
}

#[derive(Accounts)]
pub struct AcceptAdmin<'info> {
    #[account(mut)]
    pub state: Account<'info, GlobalState>,
    pub pending_admin: Signer<'info>,
}

#[derive(Accounts)]
pub struct SetEmergencyPause<'info> {
    #[account(mut, has_one = admin)]
    pub state: Account<'info, GlobalState>,
    pub admin: Signer<'info>,
}

// Global Program Account States
#[account]
pub struct GlobalState {
    pub admin: Pubkey,
    pub pending_admin: Pubkey,
    pub token_mint: Pubkey,
    pub usdt_mint: Pubkey,
    pub usdt_vault: Pubkey,
    pub reward_vault: Pubkey,
    pub presale_vault: Pubkey,
    
    pub treasury_vault: Pubkey,
    pub development_vault: Pubkey,
    pub marketing_vault: Pubkey,
    pub roadmap_vault: Pubkey,

    pub vault_authority_bump: u8,

    pub buy_active: bool,
    pub claim_active: bool,
    pub stake_active: bool,
    pub swap_active: bool,
    pub emergency_paused: bool,

    pub sell_price: u64,
    pub swap_fee_percentage: u64,
    pub transaction_count: u64,
    pub tokens_sold_presale: u64,
    pub tokens_sold_reward: u64,

    // Reward Pool accounting values
    pub reward_vault_total: u64,
    pub reward_vault_reserved: u64,
    pub reward_vault_available: u64,

    // Claims and refunds metrics
    pub treasury_claimed: u64,
    pub development_claimed: u64,
    pub marketing_claimed: u64,
    pub roadmap_claimed: u64,

    pub sale_completed: bool,
    pub reward_sale_completed: bool,

    pub sale_counter: u64,
    pub sale_transactions: u64,
    pub reward_counter: u64,
    pub returned_counter: u64,
    pub reward_paid: u64,
    pub reward_returned: u64,
    pub reward_sold: u64,

    // Launch timings
    pub treasury_start_time: i64,
    pub development_start_time: i64,
    pub marketing_start_time: i64,
    pub roadmap_start_time: i64,

    pub padding: [u8; 64], // Future growth safety padding
}

// Individual Staking Accounts
#[account]
pub struct StakingRecord {
    pub record_id: u64,
    pub user: Pubkey,
    pub staked_amount_usdt: u64,
    pub equivalent_agx: u64,
    pub total_reward_tokens: u64,
    pub released_tokens: u64,
    pub purchase_time: i64,
    pub last_claim_time: i64,
    pub lock_duration_months: u8,
    pub is_refunded: bool,
    pub is_staked: bool,
    pub padding: [u8; 32],
}

// Event System Log Specifications
#[event]
pub struct ProgramEvent {
    pub event_type: u8, // 1=Buy, 2=Claim, 3=Refund, 4=Swap, 5=OperClaim, 6=Pause, 7=Config, 8=Trans, 9=Accept
    pub user: Pubkey,
    pub amount_1: u64,
    pub amount_2: u64,
    pub record_id: u64,
}

// Error Toggles
#[error_code]
pub enum AGXError {
    #[msg("Purchase functionality is temporarily paused.")]
    PurchaseInactive,
    #[msg("Staking allocations are temporarily paused.")]
    StakingInactive,
    #[msg("Claim releases are temporarily paused.")]
    ClaimsInactive,
    #[msg("Swap pools are temporarily paused.")]
    SwapInactive,
    #[msg("Contract is under emergency pause block.")]
    EmergencyPaused,
    #[msg("Staking amount is below the 100 USDT minimum requirement.")]
    BelowMinStake,
    #[msg("Staked amount is higher than the tier limit rules.")]
    BelowMaxStake,
    #[msg("The 30,000,000 AGX hard cap limit has been reached.")]
    HardCapReached,
    #[msg("Requested refund window has expired (7 days max limit).")]
    RefundWindowExpired,
    #[msg("Operational vault release schedule is locked.")]
    OperationalTimeLocked,
    #[msg("Vault limit cap reached for the current release epoch.")]
    MaxOperationalLimitReached,
    #[msg("Specified invalid admin target address.")]
    InvalidAdminAddress,
    #[msg("Authorization constraint check failed.")]
    Unauthorized,
    #[msg("Staking record not found or active.")]
    NotStaked,
    #[msg("Refund request denied because rewards have already been claimed.")]
    RefundForbiddenClaimed,
    #[msg("This staking record has already been refunded.")]
    AlreadyRefunded,
    #[msg("Presale hard cap has already been completed.")]
    SalesCompleted,
    #[msg("Reward presale hard cap has already been completed.")]
    RewardSaleCompleted,
    #[msg("Insufficient tokens available in Reward Vault.")]
    InsufficientRewardVault,
    #[msg("Claimable balance is zero for this schedule.")]
    NoClaimableRewards,
    #[msg("Specified invalid operational vault type target.")]
    InvalidVaultType,
    #[msg("Invalid staking record ID provided.")]
    InvalidRecordId,
}
