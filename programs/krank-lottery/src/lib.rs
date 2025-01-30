use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    account_info::AccountInfo,
    // instruction::Instruction,
    program::{invoke, invoke_signed},
    pubkey::Pubkey,
    system_program,
    // system_instruction,
};
use anchor_spl::token::Token;
use solana_program::stake_history::Epoch;
// use solana_program::clock::Clock;
// use solana_program::hash::Hash;
use solana_program::sysvar::recent_blockhashes;

declare_id!("9yw8qReLMtexgYvJtndRca91NrfUeyktvMJTgANRYmbY");

#[program]

pub mod raffle_contract {

    use super::*;

    pub fn initialize_pda_lottery_vault(ctx: Context<InitializeLotteryVault>) -> Result<()> {
        let (global_pda, global_bump) =
            Pubkey::find_program_address(&b["lottery_vault"], ctx.program_id);

        if ctx.accounts.lottery_vault.lamports() > 0 {
            return Err(ErrorCode::PdaAlreadyInitialized.into());
        };

        invoke_signed(
            //creating instruction
            &system_instruction::create_account(
                ctx.accounts.payer.key,
                global_pda,
                Rent::get()?.minimum_balance(0),
                0,
                ctx.program_id,
            ),
            // Account Infos
            &[
                ctx.accounts.payer.to_account_info(),
                ctx.accounts.lottery_vault.to_account_info(),
            ],
            // PDA seeds and bump
            &[&[b"lottery_vault", &[global_bump]]],
        );

        Ok(())
    }

    pub fn initialize_lottery_counter(ctx: Context<InitializeCounter>) -> Result<()> {
        ctx.accounts.lottery_counter.total_lottery = 0;

        msg!("Lottery Counter has been initialized successfuly with total_lottery = 0");

        Ok(())
    }

    pub fn create_lottery(ctx: Context<InitializeLottery>, ticket_price: u32) -> Result<()> {
        let time_length = 604800;
        let start_time = Clock::get().unwrap().try_into().unwrap();
        let end_time = start_time + time_length;
        let lottery_id = ctx.accounts.counter.total_lottery + 1;
        let lottery = &mut ctx.accounts.Lottery;
        lottery.lottery_id = lottery_id;
        lottery.start_time = start_time;
        lottery.end_time = end_time;
        lottery.total_tickets = 0;
        lottery.winner_chosen = false;
        lottery.claimed = false;
        lottery.ticket_price = ticket_price;
        lottery.authority = ctx.accounts.user.key;

        // updating the total_lottery account
        ctx.accounts.counter.total_lottery = lottery_id;

        Ok(())
    }

    pub fn buy_ticket(ctx: Context<BuyTicket>, lottery_id: u64, ticket_price: u32) -> Result<()> {
        // Extraction of Accounts
        let lottery = &mut ctx.accounts.lottery;
        let ticket = &ctx.accounts.ticket;
        let signer = ctx.accounts.signer;
        let expiration = lottery.end_time;
        let current_time = Clock::get().unwrap();
        let global_account = ctx.accounts.lottery_vault;
        let lottery_pot = lottery.lottery_pot_amount;

        require_eq!(lottery.lottery_id, lottery_id, ErrorCode::InvalidLotteryID);
        require_eq!(
            lottery.ticket_price,
            ticket_price,
            ErrorCode::UnmatchedTicketPrice
        );
        require!(expiration <= current_time, ErrorCode::ExpiredLotterySession);

        // Transfer of ticket price from signer to lottery
        receive_sol(&ctx, &ticket_price);

        // Updating Lottery

        lottery.total_tickets += 1;
        lottery.lottery_pot_amount += ticket_price as u64;

        // Updating User Ticket
        require!(
            ctx.ticket.bump == lottery.bump,
            ErrorCode::InvalidUserTicket
        );
        require!(
            ctx.ticket.user == signer.key(),
            ErrorCode::InvalidUserTicket
        );
        require!(
            ctx.ticket.lottery == lottery.key(),
            ErrorCode::InvalidUserTicket
        );

        ctx.ticket.bump = lottery.bump;
        ctx.ticket.user = signer.key();
        ctx.ticket.lottery = lottery.key();
        ctx.ticket.lottery_number.push(lottery.total_tickets); //pushing the ticket_id to userTicket Array
        ctx.ticket.tickets_bought += 1;

        Ok(())
    }

    pub fn declare_winner(ctx: Context<DeclareWinner>, lottery_id: u64) -> Result<()> {
        let clock = Clock::get();
        let lottery = &mut ctx.accounts.lottery;

        // Require if the lottery session is over
        require_gte!(
            clock.unix_timestamp,
            lottery.end_time,
            ErrorCode::UnexpiredLotterySession
        );
        // Require if the lottery id is valid and it matches
        require_eq!(
            lottery.lottery_id,
            lottery_id,
            ErrorCode::UnmatchedLotteryId
        );

        // let recent_block_hash = .get()?
        //     .last()
        //     .map(|blockhash| blockhash.hash)
        //     .unwrap_or_default();

        let signer = ctx.accounts.signer.key();
        let lottery_id_bytes = lottery.lottery_id.to_le_bytes();

        let entropy = [
            clock.unix_timestamp.to_le_bytes(),
            recent_blockhashes.to_bytes(),
            signer.to_bytes(),
            lottery_id_bytes,
        ]
            .concat();

        // Hash the combined entropy to generate randomness
        let random_hash = solana_program::hash::Hash(&entropy);

        let random_number = u64::from_le_bytes(random_hash.to_bytes()[..8].try_into().unwrap());

        let winner = random_number % lottery.total_tickets;
        msg!(
            "The winner of the lottery {} is ticket number {}",
            lottery_id,
            winner
        );

        // Updating the lottery
        lottery.winner_choosen = true;
        lottery.winner = winner;

        charge_fees(ctx);

        Ok(())
    }

    pub fn claim_prize(ctx: Context<ClaimPrize>, lottery_id: u64) -> Result<()> {
        let lottery = &mut ctx.accounts.lottery;
        let user_ticket = ctx.accounts.ticket;

        require_eq!(lottery.lottery_id, lottery_id);
        for ticket in user_ticket.lottery_number {
            require_eq!(lottery.winner, ticket);
            transfer_sol(
                &ctx,
                &ctx.accounts.signer.key(),
                &lottery.lottery_pot_amount,
            )?;
        }
    }

    //

    fn charge_fees(&ctx: Context<DeclareWinner>, &recipient: Pubkey) -> Result<()> {
        let lottery = &mut ctx.lottery;
        let fee: u64 = (lottery.lottery_pot_amount * 10) / 100;

        let signer = &ctx.accounts.signer;
        let vault = &ctx.accounts.vault;

        // Verify the vault PDA
        let (pda, _bump) =
            Pubkey::find_program_address(&[b"vault", signer.key().as_ref()], ctx.program_id);

        require_keys_eq!(vault.key(), pda, "Invalid vault PDA.");

        require!(
            **vault.lamports.borrow() >= fee,
            ErrorCode::InsufficientVaultBalance
        );

        // Performing Transfer
        transfer_sol(&ctx, &recipient, &fee);
        // Updating Lottery Pot Amount
        lottery.lottery_pot_amount -= fee;

        Ok(())
    }

    fn transfer_sol<T>(&ctx: Context<T>, &recipient: Pubkey, &amount: u64) -> Result<()> {
        let vault = ctx.accounts.vault;

        **vault.to_account_info().try_borrow_mut_lamports()? -= amount;
        let recipient_account = AccountInfo::new(
            &recipient,          // The recipient's public key
            false,               // Recipient is not a signer
            true,                // Account is writable
            &mut 0,              // Placeholder for the lamports balance
            &mut [],             // Placeholder for account data
            &system_program::ID, // Owned by the system program
            false,               // Not an executable account
            Epoch::default(),    // Placeholder for the rent epoch
        );
        **recipient_account.try_borrow_mut_lamports()? += amount;

        msg!(
            "Transferred {} lamports from vault {} to recipient {}.",
            amount,
            vault.key(),
            recipient.key()
        );

        Ok(())
    }

    fn receive_sol<T>(&ctx: Context<T>, &ticket_price: u64) -> Result<()> {
        let signer = ctx.accounts.signer;
        let global_account = ctx.accounts.vault;

        let transfer_instruction = anchor_lang::solana_program::system_instruction::transfer(
            &signer.key(),
            global_account.key(),
            ticket_price.into(),
        );

        anchor_lang::solana_program::program::invoke(
            &transfer_instruction,
            &[
                signer.to_account_info(),
                global_account.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        msg!(
            "{} has been transfered to vault for {} ",
            ticket_price,
            signer.key()
        );

        Ok(())
    }

}

// Delcaration of Interaction Interface

#[derive(Accounts)]
pub struct InitializeLotteryVault<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        init_if_needed,
        seeds = [b"vault", payer.key().as_ref()],
        bump,
        payer = payer,
        space = 8 + 32 // Space for account metadata (adjust if needed)
    )]
    pub lottery_vault: Account<'info, LotteryVault>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct InitializeCounter<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,

    #[account(
        init,
        payer=signer,
        space = 8 + LotteryCounter::INIT_SPACE,
        seeds=[b"lottery_counter".as_ref()],
        bump,
    )]
    pub lottery_counter: Account<'info, LotteryCounter>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(lottery_id:u64)]
pub struct InitializeLottery<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,

    #[account(
        init,
        payer= signer,
        space= 8 + Lottery::INIT_SPACE,
        seeds=[b"lottery",lottery_id.to_le_bytes().as_ref()],
        bump,

    )]
    pub lottery: Account<'info, Lottery>,

    pub counter: Account<'info, LotteryCounter>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct BuyTicket<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,

    #[account(mut)]
    pub lottery: Account<'info, Lottery>,

    pub lottery_vault: Account<'info, LotteryVault>,

    #[account(
        init_if_needed,
        payer = signer,
        space = 8 + UserTicket::INIT_SPACE,
        seeds = [b"ticket",lottery.key().as_ref(), signer.key().as_ref()],
        bump
    )]
    pub ticket: Account<'info, UserTicket>,

    // Token transfer programs
    #[account(address= spl_token::id())]
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct DeclareWinner<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,

    #[account(mut)]
    pub lottery: Account<'info, Lottery>,

    #[account(
        mut,
        seeds = [b"vault", signer.key().as_ref()],
        bump,
    )]
    pub vault: SystemAccount<'info>,

    pub system_program: Program<'info, System>,

    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct ClaimPrize<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,

    #[account(mut)]
    pub lottery: Account<'info, Lottery>,

    #[account()]
    pub ticket: Account<'info, UserTicket>,

    #[account(
        mut,
        seeds = [b"vault", signer.key().as_ref()],
        bump,
    )]
    pub vault: SystemAccount<'info, LotteryVault>,

    pub system_program: Program<'info, System>,
}

// #[derive(Accounts)]
// pub struct ChargeFees<'Info> {
//     #[account(mut)]
//     pub declare_winner: DeclareWinner<'Info>,

//     pub system_program: Program<'info, System>,
// }

// Storage Accounts

#[account]
#[derive(InitSpace)]
pub struct Lottery {
    pub bump: u8,
    pub lottery_id: u64,
    pub start_time: u64,
    pub end_time: u64,
    pub total_tickets: u32,
    pub ticket_price: u32,
    pub lottery_pot_amount: u64,
    pub winner: Option<u64>,
    pub winner_chosen: bool,
    pub claimed: bool,
    pub authority: Pubkey,
}

#[account]
pub struct LotteryVault {
    amount: u64,
}

#[account]
pub struct UserTicket {
    pub bump: u8,
    pub lottery: Pubkey,
    pub user: Pubkey,
    pub lottery_number: Vec<u64>,
    pub tickets_bought: u32,
}

#[account]
pub struct LotteryCounter {
    pub total_lottery: u64,
}

// Implementation of Structs

impl UserTicket {
    pub const INIT_SPACE: usize = 32 + 32 + 8;
}
impl LotteryCounter {
    pub const INIT_SPACE: usize = 8 + 8;
}

// Error Checks

#[error_code]
pub enum ErrorCode {
    #[msg("The PDA account has already been initialized")]
    PdaAlreadyInitialized,

    #[msg("Invalid Lottery Id")]
    InvalidLotteryId,

    #[msg("Umatched Lottery Id")]
    UnmatchedLotteryId,

    #[msg("Not the ticket price")]
    UnmatchedTicketPrice,

    #[msg("The lottery session has expired")]
    ExpiredLotterySession,

    #[msg("Invalid UserTicket")]
    InvalidUserTicket,

    #[msg("Insufficient Vault Balance")]
    InsufficientVaultBalance,

    #[msg("The lottery session isn't expired")]
    UnexpiredLotterySession,
}

