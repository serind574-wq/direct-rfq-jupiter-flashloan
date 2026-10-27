use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{self, Mint, Token, TokenAccount, Transfer},
};

// Replace after deployment with: `anchor keys list`, then update this value and Anchor.toml.
declare_id!("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS");

/// Educational direct-RFQ settlement program.
///
/// Lifecycle:
/// 1. Authority registers a fixed maker/pair and PDA-controlled input/output vaults.
/// 2. Taker creates an RFQ on-chain for exactly one maker/pair.
/// 3. The maker posts one executable on-chain quote, reserving output inventory.
/// 4. A client composes `fill_quote` with a flash borrow/payback transaction.
///
/// This program does NOT borrow from Jupiter Lend. Jupiter's SDK creates top-level
/// borrow/payback instructions in the client transaction. This program supplies the
/// two composable RFQ fills placed between them.
#[program]
pub mod direct_rfq_arb {
    use super::*;

    pub fn initialize_protocol(
        ctx: Context<InitializeProtocol>,
        max_request_ttl_slots: u64,
    ) -> Result<()> {
        require!(max_request_ttl_slots > 0, RfqError::InvalidTtl);

        let config = &mut ctx.accounts.config;
        config.authority = ctx.accounts.authority.key();
        config.max_request_ttl_slots = max_request_ttl_slots;
        config.paused = false;
        config.bump = ctx.bumps.config;
        Ok(())
    }

    /// Registers one fixed direction: input mint -> output mint.
    ///
    /// The maker must fund `maker_output_vault` before posting quotes. The pair also
    /// has an input vault where filled taker input accumulates as maker proceeds.
    pub fn register_maker_pair(
        ctx: Context<RegisterMakerPair>,
        max_input_amount: u64,
    ) -> Result<()> {
        require!(!ctx.accounts.config.paused, RfqError::ProtocolPaused);
        require!(max_input_amount > 0, RfqError::InvalidAmount);
        require!(
            ctx.accounts.input_mint.key() != ctx.accounts.output_mint.key(),
            RfqError::SameMint
        );

        let pair = &mut ctx.accounts.maker_pair;
        pair.maker_owner = ctx.accounts.maker_owner.key();
        pair.input_mint = ctx.accounts.input_mint.key();
        pair.output_mint = ctx.accounts.output_mint.key();
        pair.input_vault = ctx.accounts.maker_input_vault.key();
        pair.output_vault = ctx.accounts.maker_output_vault.key();
        pair.max_input_amount = max_input_amount;
        pair.reserved_output_amount = 0;
        pair.enabled = true;
        pair.paused = false;
        pair.bump = ctx.bumps.maker_pair;
        pair.vault_authority_bump = ctx.bumps.vault_authority;
        Ok(())
    }

    /// Maker deposits the asset that it will deliver to takers for this pair.
    pub fn deposit_output_inventory(
        ctx: Context<DepositOutputInventory>,
        amount: u64,
    ) -> Result<()> {
        require!(amount > 0, RfqError::InvalidAmount);
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.maker_output_source.to_account_info(),
                    to: ctx.accounts.maker_output_vault.to_account_info(),
                    authority: ctx.accounts.maker_owner.to_account_info(),
                },
            ),
            amount,
        )?;
        Ok(())
    }

    /// Maker can withdraw only inventory that is not reserved by an open quote.
    pub fn withdraw_output_inventory(
        ctx: Context<WithdrawOutputInventory>,
        amount: u64,
    ) -> Result<()> {
        require!(amount > 0, RfqError::InvalidAmount);
        let free_inventory = ctx
            .accounts
            .maker_output_vault
            .amount
            .checked_sub(ctx.accounts.maker_pair.reserved_output_amount)
            .ok_or(RfqError::ReservationInvariantBroken)?;
        require!(amount <= free_inventory, RfqError::InventoryReserved);

        let pair_key = ctx.accounts.maker_pair.key();
        let signer_seeds: &[&[&[u8]]] = &[&[
            b"vault-authority",
            pair_key.as_ref(),
            &[ctx.accounts.maker_pair.vault_authority_bump],
        ]];
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.maker_output_vault.to_account_info(),
                    to: ctx.accounts.maker_output_destination.to_account_info(),
                    authority: ctx.accounts.vault_authority.to_account_info(),
                },
                signer_seeds,
            ),
            amount,
        )?;
        Ok(())
    }

    /// Maker withdraws the asset paid by takers after one or more fills.
    pub fn withdraw_input_proceeds(
        ctx: Context<WithdrawInputProceeds>,
        amount: u64,
    ) -> Result<()> {
        require!(amount > 0, RfqError::InvalidAmount);
        let pair_key = ctx.accounts.maker_pair.key();
        let signer_seeds: &[&[&[u8]]] = &[&[
            b"vault-authority",
            pair_key.as_ref(),
            &[ctx.accounts.maker_pair.vault_authority_bump],
        ]];
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.maker_input_vault.to_account_info(),
                    to: ctx.accounts.maker_input_destination.to_account_info(),
                    authority: ctx.accounts.vault_authority.to_account_info(),
                },
                signer_seeds,
            ),
            amount,
        )?;
        Ok(())
    }

    /// Taker creates an RFQ before its later flash-loan transaction.
    ///
    /// The request locks a single pair and exact input amount. The maker must quote
    /// at or above `min_output_amount` and cannot quote beyond available inventory.
    pub fn create_rfq(ctx: Context<CreateRfq>, args: CreateRfqArgs) -> Result<()> {
        require!(!ctx.accounts.config.paused, RfqError::ProtocolPaused);
        require!(ctx.accounts.maker_pair.enabled, RfqError::MakerPairDisabled);
        require!(!ctx.accounts.maker_pair.paused, RfqError::MakerPairPaused);
        require!(args.input_amount > 0, RfqError::InvalidAmount);
        require!(
            args.input_amount <= ctx.accounts.maker_pair.max_input_amount,
            RfqError::InputAmountExceedsLimit
        );

        let current_slot = Clock::get()?.slot;
        require!(args.expiry_slot > current_slot, RfqError::InvalidTtl);
        let maximum_expiry = current_slot
            .checked_add(ctx.accounts.config.max_request_ttl_slots)
            .ok_or(RfqError::MathOverflow)?;
        require!(args.expiry_slot <= maximum_expiry, RfqError::TtlTooLong);

        let rfq = &mut ctx.accounts.rfq;
        rfq.taker = ctx.accounts.taker.key();
        rfq.maker_pair = ctx.accounts.maker_pair.key();
        rfq.taker_input_account = ctx.accounts.taker_input_account.key();
        rfq.taker_output_account = ctx.accounts.taker_output_account.key();
        rfq.request_id = args.request_id;
        rfq.input_amount = args.input_amount;
        rfq.min_output_amount = args.min_output_amount;
        rfq.expiry_slot = args.expiry_slot;
        rfq.status = RfqStatus::Open;
        rfq.bump = ctx.bumps.rfq;
        Ok(())
    }

    /// The fixed maker accepts and quotes an existing RFQ on-chain. This is the
    /// direct on-chain quote stage, deliberately separate from the flash transaction.
    pub fn post_quote(ctx: Context<PostQuote>, args: PostQuoteArgs) -> Result<()> {
        require!(!ctx.accounts.config.paused, RfqError::ProtocolPaused);
        require!(ctx.accounts.maker_pair.enabled, RfqError::MakerPairDisabled);
        require!(!ctx.accounts.maker_pair.paused, RfqError::MakerPairPaused);
        require!(ctx.accounts.rfq.status == RfqStatus::Open, RfqError::RfqNotOpen);
        require!(args.output_amount > 0, RfqError::InvalidAmount);
        require!(args.output_amount >= ctx.accounts.rfq.min_output_amount, RfqError::OutputBelowMinimum);

        let current_slot = Clock::get()?.slot;
        require!(current_slot <= ctx.accounts.rfq.expiry_slot, RfqError::RfqExpired);
        require!(args.expiry_slot > current_slot, RfqError::InvalidTtl);
        require!(args.expiry_slot <= ctx.accounts.rfq.expiry_slot, RfqError::QuoteExpiryAfterRfq);

        let free_inventory = ctx
            .accounts
            .maker_output_vault
            .amount
            .checked_sub(ctx.accounts.maker_pair.reserved_output_amount)
            .ok_or(RfqError::ReservationInvariantBroken)?;
        require!(args.output_amount <= free_inventory, RfqError::InsufficientFreeInventory);

        let pair = &mut ctx.accounts.maker_pair;
        pair.reserved_output_amount = pair
            .reserved_output_amount
            .checked_add(args.output_amount)
            .ok_or(RfqError::MathOverflow)?;

        let quote = &mut ctx.accounts.quote;
        quote.rfq = ctx.accounts.rfq.key();
        quote.maker_pair = ctx.accounts.maker_pair.key();
        quote.output_amount = args.output_amount;
        quote.expiry_slot = args.expiry_slot;
        quote.status = QuoteStatus::Open;
        quote.bump = ctx.bumps.quote;
        Ok(())
    }

    /// Releases a reserved quote before it is filled. The quote cannot be reposted;
    /// the taker should create a new RFQ with a new request ID.
    pub fn cancel_quote(ctx: Context<CancelQuote>) -> Result<()> {
        require!(ctx.accounts.quote.status == QuoteStatus::Open, RfqError::QuoteNotOpen);
        release_reservation(&mut ctx.accounts.maker_pair, ctx.accounts.quote.output_amount)?;
        ctx.accounts.quote.status = QuoteStatus::Cancelled;
        Ok(())
    }

    /// Anyone may release a quote after expiry. This prevents stale reservations from
    /// blocking maker withdrawals forever.
    pub fn expire_quote(ctx: Context<ExpireQuote>) -> Result<()> {
        require!(ctx.accounts.quote.status == QuoteStatus::Open, RfqError::QuoteNotOpen);
        require!(Clock::get()?.slot > ctx.accounts.quote.expiry_slot, RfqError::QuoteStillLive);
        release_reservation(&mut ctx.accounts.maker_pair, ctx.accounts.quote.output_amount)?;
        ctx.accounts.quote.status = QuoteStatus::Expired;
        Ok(())
    }

    /// Composable settlement instruction. Put this between Jupiter Lend's borrow and
    /// payback instructions. No off-chain signature is checked here because the maker
    /// already signed its `post_quote` transaction and the quote is stored on-chain.
    pub fn fill_quote(ctx: Context<FillQuote>) -> Result<()> {
        require!(!ctx.accounts.config.paused, RfqError::ProtocolPaused);
        require!(ctx.accounts.maker_pair.enabled, RfqError::MakerPairDisabled);
        require!(!ctx.accounts.maker_pair.paused, RfqError::MakerPairPaused);
        require!(ctx.accounts.rfq.status == RfqStatus::Open, RfqError::RfqNotOpen);
        require!(ctx.accounts.quote.status == QuoteStatus::Open, RfqError::QuoteNotOpen);

        let current_slot = Clock::get()?.slot;
        require!(current_slot <= ctx.accounts.rfq.expiry_slot, RfqError::RfqExpired);
        require!(current_slot <= ctx.accounts.quote.expiry_slot, RfqError::QuoteExpired);

        // First leg: the flash-borrowed/taker input is paid to the fixed maker's vault.
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.taker_input_account.to_account_info(),
                    to: ctx.accounts.maker_input_vault.to_account_info(),
                    authority: ctx.accounts.taker.to_account_info(),
                },
            ),
            ctx.accounts.rfq.input_amount,
        )?;

        // Second leg: the PDA-controlled maker inventory is paid to the taker.
        let pair_key = ctx.accounts.maker_pair.key();
        let signer_seeds: &[&[&[u8]]] = &[&[
            b"vault-authority",
            pair_key.as_ref(),
            &[ctx.accounts.maker_pair.vault_authority_bump],
        ]];
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.maker_output_vault.to_account_info(),
                    to: ctx.accounts.taker_output_account.to_account_info(),
                    authority: ctx.accounts.vault_authority.to_account_info(),
                },
                signer_seeds,
            ),
            ctx.accounts.quote.output_amount,
        )?;

        release_reservation(&mut ctx.accounts.maker_pair, ctx.accounts.quote.output_amount)?;
        ctx.accounts.quote.status = QuoteStatus::Filled;
        ctx.accounts.rfq.status = RfqStatus::Filled;
        Ok(())
    }

    /// Place after two RFQ fills and before Jupiter's payback instruction. For a clean
    /// experiment, use a dedicated input token account and set min_amount to:
    /// initial_balance + flash_amount + desired_profit.
    pub fn assert_min_token_balance(
        ctx: Context<AssertMinTokenBalance>,
        min_amount: u64,
    ) -> Result<()> {
        require!(
            ctx.accounts.token_account.amount >= min_amount,
            RfqError::ProfitFloorNotMet
        );
        Ok(())
    }

    pub fn set_protocol_pause(ctx: Context<SetProtocolPause>, paused: bool) -> Result<()> {
        ctx.accounts.config.paused = paused;
        Ok(())
    }
}

fn release_reservation(pair: &mut Account<MakerPairConfig>, amount: u64) -> Result<()> {
    pair.reserved_output_amount = pair
        .reserved_output_amount
        .checked_sub(amount)
        .ok_or(RfqError::ReservationInvariantBroken)?;
    Ok(())
}

#[derive(Accounts)]
pub struct InitializeProtocol<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        init,
        payer = authority,
        space = ProtocolConfig::SPACE,
        seeds = [b"protocol"],
        bump
    )]
    pub config: Account<'info, ProtocolConfig>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RegisterMakerPair<'info> {
    #[account(
        seeds = [b"protocol"],
        bump = config.bump,
        has_one = authority
    )]
    pub config: Account<'info, ProtocolConfig>,
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(mut)]
    pub maker_owner: Signer<'info>,
    pub input_mint: Account<'info, Mint>,
    pub output_mint: Account<'info, Mint>,
    #[account(
        init,
        payer = authority,
        space = MakerPairConfig::SPACE,
        seeds = [
            b"maker-pair",
            maker_owner.key().as_ref(),
            input_mint.key().as_ref(),
            output_mint.key().as_ref(),
        ],
        bump
    )]
    pub maker_pair: Account<'info, MakerPairConfig>,
    /// CHECK: PDA only; no data is read or written.
    #[account(
        seeds = [b"vault-authority", maker_pair.key().as_ref()],
        bump
    )]
    pub vault_authority: UncheckedAccount<'info>,
    #[account(
        init,
        payer = authority,
        associated_token::mint = input_mint,
        associated_token::authority = vault_authority
    )]
    pub maker_input_vault: Account<'info, TokenAccount>,
    #[account(
        init,
        payer = authority,
        associated_token::mint = output_mint,
        associated_token::authority = vault_authority
    )]
    pub maker_output_vault: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct DepositOutputInventory<'info> {
    #[account(
        has_one = maker_owner,
        constraint = maker_pair.enabled @ RfqError::MakerPairDisabled
    )]
    pub maker_pair: Account<'info, MakerPairConfig>,
    pub maker_owner: Signer<'info>,
    #[account(
        mut,
        constraint = maker_output_source.owner == maker_owner.key() @ RfqError::WrongTokenOwner,
        constraint = maker_output_source.mint == maker_pair.output_mint @ RfqError::WrongMint
    )]
    pub maker_output_source: Account<'info, TokenAccount>,
    #[account(
        mut,
        address = maker_pair.output_vault @ RfqError::WrongVault
    )]
    pub maker_output_vault: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct WithdrawOutputInventory<'info> {
    #[account(mut, has_one = maker_owner)]
    pub maker_pair: Account<'info, MakerPairConfig>,
    pub maker_owner: Signer<'info>,
    #[account(
        mut,
        address = maker_pair.output_vault @ RfqError::WrongVault
    )]
    pub maker_output_vault: Account<'info, TokenAccount>,
    #[account(
        mut,
        constraint = maker_output_destination.owner == maker_owner.key() @ RfqError::WrongTokenOwner,
        constraint = maker_output_destination.mint == maker_pair.output_mint @ RfqError::WrongMint
    )]
    pub maker_output_destination: Account<'info, TokenAccount>,
    /// CHECK: PDA signer verified by seeds.
    #[account(
        seeds = [b"vault-authority", maker_pair.key().as_ref()],
        bump = maker_pair.vault_authority_bump
    )]
    pub vault_authority: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct WithdrawInputProceeds<'info> {
    #[account(has_one = maker_owner)]
    pub maker_pair: Account<'info, MakerPairConfig>,
    pub maker_owner: Signer<'info>,
    #[account(
        mut,
        address = maker_pair.input_vault @ RfqError::WrongVault
    )]
    pub maker_input_vault: Account<'info, TokenAccount>,
    #[account(
        mut,
        constraint = maker_input_destination.owner == maker_owner.key() @ RfqError::WrongTokenOwner,
        constraint = maker_input_destination.mint == maker_pair.input_mint @ RfqError::WrongMint
    )]
    pub maker_input_destination: Account<'info, TokenAccount>,
    /// CHECK: PDA signer verified by seeds.
    #[account(
        seeds = [b"vault-authority", maker_pair.key().as_ref()],
        bump = maker_pair.vault_authority_bump
    )]
    pub vault_authority: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct CreateRfqArgs {
    pub request_id: u64,
    pub input_amount: u64,
    pub min_output_amount: u64,
    pub expiry_slot: u64,
}

#[derive(Accounts)]
#[instruction(args: CreateRfqArgs)]
pub struct CreateRfq<'info> {
    #[account(
        seeds = [b"protocol"],
        bump = config.bump
    )]
    pub config: Account<'info, ProtocolConfig>,
    #[account(
        constraint = maker_pair.enabled @ RfqError::MakerPairDisabled
    )]
    pub maker_pair: Account<'info, MakerPairConfig>,
    #[account(mut)]
    pub taker: Signer<'info>,
    #[account(
        constraint = taker_input_account.owner == taker.key() @ RfqError::WrongTokenOwner,
        constraint = taker_input_account.mint == maker_pair.input_mint @ RfqError::WrongMint
    )]
    pub taker_input_account: Account<'info, TokenAccount>,
    #[account(
        constraint = taker_output_account.owner == taker.key() @ RfqError::WrongTokenOwner,
        constraint = taker_output_account.mint == maker_pair.output_mint @ RfqError::WrongMint
    )]
    pub taker_output_account: Account<'info, TokenAccount>,
    #[account(
        init,
        payer = taker,
        space = RfqRequest::SPACE,
        seeds = [b"rfq", taker.key().as_ref(), &args.request_id.to_le_bytes()],
        bump
    )]
    pub rfq: Account<'info, RfqRequest>,
    pub system_program: Program<'info, System>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct PostQuoteArgs {
    pub output_amount: u64,
    pub expiry_slot: u64,
}

#[derive(Accounts)]
#[instruction(args: PostQuoteArgs)]
pub struct PostQuote<'info> {
    #[account(seeds = [b"protocol"], bump = config.bump)]
    pub config: Account<'info, ProtocolConfig>,
    #[account(mut, has_one = maker_owner)]
    pub maker_pair: Account<'info, MakerPairConfig>,
    #[account(mut)]
    pub maker_owner: Signer<'info>,
    #[account(
        mut,
        seeds = [b"rfq", rfq.taker.as_ref(), &rfq.request_id.to_le_bytes()],
        bump = rfq.bump,
        constraint = rfq.maker_pair == maker_pair.key() @ RfqError::WrongMakerPair
    )]
    pub rfq: Account<'info, RfqRequest>,
    #[account(
        init,
        payer = maker_owner,
        space = Quote::SPACE,
        seeds = [b"quote", rfq.key().as_ref()],
        bump
    )]
    pub quote: Account<'info, Quote>,
    #[account(
        address = maker_pair.output_vault @ RfqError::WrongVault,
        constraint = maker_output_vault.mint == maker_pair.output_mint @ RfqError::WrongMint
    )]
    pub maker_output_vault: Account<'info, TokenAccount>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CancelQuote<'info> {
    #[account(mut, has_one = maker_owner)]
    pub maker_pair: Account<'info, MakerPairConfig>,
    pub maker_owner: Signer<'info>,
    #[account(
        mut,
        constraint = quote.maker_pair == maker_pair.key() @ RfqError::WrongMakerPair
    )]
    pub quote: Account<'info, Quote>,
}

#[derive(Accounts)]
pub struct ExpireQuote<'info> {
    #[account(mut)]
    pub maker_pair: Account<'info, MakerPairConfig>,
    #[account(
        mut,
        constraint = quote.maker_pair == maker_pair.key() @ RfqError::WrongMakerPair
    )]
    pub quote: Account<'info, Quote>,
    pub caller: Signer<'info>,
}

#[derive(Accounts)]
pub struct FillQuote<'info> {
    #[account(seeds = [b"protocol"], bump = config.bump)]
    pub config: Account<'info, ProtocolConfig>,
    #[account(mut)]
    pub maker_pair: Account<'info, MakerPairConfig>,
    #[account(mut)]
    pub taker: Signer<'info>,
    #[account(
        mut,
        seeds = [b"rfq", rfq.taker.as_ref(), &rfq.request_id.to_le_bytes()],
        bump = rfq.bump,
        constraint = rfq.taker == taker.key() @ RfqError::WrongTaker,
        constraint = rfq.maker_pair == maker_pair.key() @ RfqError::WrongMakerPair
    )]
    pub rfq: Account<'info, RfqRequest>,
    #[account(
        mut,
        seeds = [b"quote", rfq.key().as_ref()],
        bump = quote.bump,
        constraint = quote.rfq == rfq.key() @ RfqError::WrongRfq,
        constraint = quote.maker_pair == maker_pair.key() @ RfqError::WrongMakerPair
    )]
    pub quote: Account<'info, Quote>,
    #[account(
        mut,
        address = rfq.taker_input_account @ RfqError::WrongTakerTokenAccount,
        constraint = taker_input_account.owner == taker.key() @ RfqError::WrongTokenOwner,
        constraint = taker_input_account.mint == maker_pair.input_mint @ RfqError::WrongMint
    )]
    pub taker_input_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        address = rfq.taker_output_account @ RfqError::WrongTakerTokenAccount,
        constraint = taker_output_account.owner == taker.key() @ RfqError::WrongTokenOwner,
        constraint = taker_output_account.mint == maker_pair.output_mint @ RfqError::WrongMint
    )]
    pub taker_output_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        address = maker_pair.input_vault @ RfqError::WrongVault,
        constraint = maker_input_vault.mint == maker_pair.input_mint @ RfqError::WrongMint
    )]
    pub maker_input_vault: Account<'info, TokenAccount>,
    #[account(
        mut,
        address = maker_pair.output_vault @ RfqError::WrongVault,
        constraint = maker_output_vault.mint == maker_pair.output_mint @ RfqError::WrongMint
    )]
    pub maker_output_vault: Account<'info, TokenAccount>,
    /// CHECK: PDA signer verified by seeds.
    #[account(
        seeds = [b"vault-authority", maker_pair.key().as_ref()],
        bump = maker_pair.vault_authority_bump
    )]
    pub vault_authority: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct AssertMinTokenBalance<'info> {
    pub taker: Signer<'info>,
    #[account(
        constraint = token_account.owner == taker.key() @ RfqError::WrongTokenOwner
    )]
    pub token_account: Account<'info, TokenAccount>,
}

#[derive(Accounts)]
pub struct SetProtocolPause<'info> {
    #[account(
        mut,
        seeds = [b"protocol"],
        bump = config.bump,
        has_one = authority
    )]
    pub config: Account<'info, ProtocolConfig>,
    pub authority: Signer<'info>,
}

#[account]
pub struct ProtocolConfig {
    pub authority: Pubkey,
    pub max_request_ttl_slots: u64,
    pub paused: bool,
    pub bump: u8,
}
impl ProtocolConfig {
    pub const SPACE: usize = 8 + 32 + 8 + 1 + 1;
}

#[account]
pub struct MakerPairConfig {
    pub maker_owner: Pubkey,
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub input_vault: Pubkey,
    pub output_vault: Pubkey,
    pub max_input_amount: u64,
    pub reserved_output_amount: u64,
    pub enabled: bool,
    pub paused: bool,
    pub bump: u8,
    pub vault_authority_bump: u8,
}
impl MakerPairConfig {
    pub const SPACE: usize = 8 + (32 * 5) + (8 * 2) + 4;
}

#[account]
pub struct RfqRequest {
    pub taker: Pubkey,
    pub maker_pair: Pubkey,
    pub taker_input_account: Pubkey,
    pub taker_output_account: Pubkey,
    pub request_id: u64,
    pub input_amount: u64,
    pub min_output_amount: u64,
    pub expiry_slot: u64,
    pub status: RfqStatus,
    pub bump: u8,
}
impl RfqRequest {
    pub const SPACE: usize = 8 + (32 * 4) + (8 * 4) + 2;
}

#[account]
pub struct Quote {
    pub rfq: Pubkey,
    pub maker_pair: Pubkey,
    pub output_amount: u64,
    pub expiry_slot: u64,
    pub status: QuoteStatus,
    pub bump: u8,
}
impl Quote {
    pub const SPACE: usize = 8 + (32 * 2) + (8 * 2) + 2;
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq)]
pub enum RfqStatus {
    Open,
    Filled,
    Cancelled,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq)]
pub enum QuoteStatus {
    Open,
    Filled,
    Cancelled,
    Expired,
}

#[error_code]
pub enum RfqError {
    #[msg("Protocol is paused")]
    ProtocolPaused,
    #[msg("Maker pair is disabled")]
    MakerPairDisabled,
    #[msg("Maker pair is paused")]
    MakerPairPaused,
    #[msg("Input and output mints must be different")]
    SameMint,
    #[msg("Amount must be positive")]
    InvalidAmount,
    #[msg("Invalid quote or RFQ TTL")]
    InvalidTtl,
    #[msg("Requested TTL is above protocol limit")]
    TtlTooLong,
    #[msg("Integer arithmetic overflow")]
    MathOverflow,
    #[msg("Input amount is above maker pair limit")]
    InputAmountExceedsLimit,
    #[msg("RFQ is not open")]
    RfqNotOpen,
    #[msg("RFQ has expired")]
    RfqExpired,
    #[msg("Quote is not open")]
    QuoteNotOpen,
    #[msg("Quote has expired")]
    QuoteExpired,
    #[msg("Quote has not expired")]
    QuoteStillLive,
    #[msg("Quote output is below RFQ minimum output")]
    OutputBelowMinimum,
    #[msg("Quote expires after RFQ")]
    QuoteExpiryAfterRfq,
    #[msg("Insufficient free maker output inventory")]
    InsufficientFreeInventory,
    #[msg("Maker inventory is reserved by open quotes")]
    InventoryReserved,
    #[msg("Reserved inventory invariant is broken")]
    ReservationInvariantBroken,
    #[msg("Wrong token mint")]
    WrongMint,
    #[msg("Wrong token account owner")]
    WrongTokenOwner,
    #[msg("Wrong vault account")]
    WrongVault,
    #[msg("Wrong fixed maker pair")]
    WrongMakerPair,
    #[msg("Wrong RFQ account")]
    WrongRfq,
    #[msg("Wrong taker")]
    WrongTaker,
    #[msg("Wrong taker token account")]
    WrongTakerTokenAccount,
    #[msg("Final flash asset balance is below required profit floor")]
    ProfitFloorNotMet,
}
