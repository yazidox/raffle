use anchor_lang::prelude::*;
use anchor_spl::token::{self};

pub mod contexts;
pub mod utils;
pub mod constants;
pub mod account;
pub mod errors;

use contexts::*;
use utils::*;
use errors::*;
use constants::*;

declare_id!("DYUw8M1XatBe8NTAqJRuVWFVfFbS2GMEesS68wMsokDH");

#[program]
pub mod raffle {
    use super::*;
    use anchor_lang::AccountsClose;

    pub fn create_raffle(
        ctx: Context<CreateRaffleContext>, 
        raffle_id: u64,
        start_time: u32,
        end_time: u32,
        total_ticket: Option<u32>,
        price: u64,
        min_nft_count: u32
    ) -> Result<()> {
        let mut a_pool = ctx.accounts.pool.load_init()?;

        require!(
            start_time < end_time,
            RaffleError::StartedRaffle
        );
        let a_mint = &ctx.accounts.mint;

        a_pool.raffle_id = raffle_id;
        a_pool.start_time = start_time;
        a_pool.end_time = end_time;
        a_pool.mint = a_mint.to_account_info().key();
        a_pool.ticket_price = price;
        a_pool.count = 0;
        if total_ticket.is_some() {
            a_pool.total_ticket = total_ticket.unwrap();
        }
        else {
            a_pool.total_ticket = MAX_TOTAL_TICKET;
        }
        a_pool.purchased_ticket = 0;
        a_pool.state = 0;
        a_pool.min_nft_count = min_nft_count;
        token::transfer(ctx.accounts.transfer_context(), 1)?;
        Ok(())
    }

    pub fn edit_raffle(
        ctx: Context<EditRaffleContext>, 
        start_time: u32,
        end_time: u32, 
        total_ticket: Option<u32>,
        price: u64,
        min_nft_count: u32
    ) -> Result<()> {
        let mut a_pool = ctx.accounts.pool.load_mut()?;
        let current_time = get_current_time()?;

        require!(
            start_time < end_time,
            RaffleError::StartedRaffle
        );

        require!(
            current_time < a_pool.start_time,
            RaffleError::StartedRaffle
        );

        a_pool.start_time = start_time;
        a_pool.end_time = end_time;
        if total_ticket.is_some() {
            a_pool.total_ticket = total_ticket.unwrap();
        }
        else {
            a_pool.total_ticket = MAX_TOTAL_TICKET;
        }
        a_pool.ticket_price = price;
        a_pool.min_nft_count = min_nft_count;

        Ok(())
    }

    pub fn delete_raffle(
        ctx: Context<DeleteRaffleContext>
    ) -> Result<()> {
        {

            let a_pool = ctx.accounts.pool.load()?;
            
            let current_time = get_current_time()?;
            require!(
                current_time < a_pool.start_time || (current_time > a_pool.end_time && a_pool.purchased_ticket == 0  ) ,
                RaffleError::StartedRaffle
            );
    
            let (_pool, bump) = Pubkey::find_program_address(
                &[POOL_SEED.as_ref(), 
                &a_pool.raffle_id.to_le_bytes(), 
                a_pool.mint.key().as_ref()], 
                ctx.program_id
            );
            
            let seeds = &[POOL_SEED.as_bytes(), &a_pool.raffle_id.to_le_bytes(), a_pool.mint.as_ref(), &[bump]];
            let signer = &[&seeds[..]];
    
            token::transfer(
                ctx.accounts.transfer_context().with_signer(signer), 
                1
            )?;
        }

        {
            let a_admin = &ctx.accounts.admin;
            ctx.accounts.pool.close(a_admin.to_account_info())?;
            Ok(())
        }
    }

    pub fn buy_ticket(ctx: Context<BuyTicketContext>, amount: u32, nft_count: u32) -> Result<()> {
        let mut a_pool = ctx.accounts.pool.load_mut()?;
        let a_buyer = &ctx.accounts.buyer;

        let current_time = get_current_time()?;
        let total_ticket = a_pool.total_ticket;

        let m_data = &mut ctx.accounts.metadata.try_borrow_data()?;
        let metadata = mpl_token_metadata::state::Metadata::deserialize(&mut &m_data[..])?;

        //Verify Collection

        let collection_not_proper = metadata
            .data
            .creators
            .as_ref()
            .unwrap()
            .iter()
            .filter(|item| COLLECTION_KEY == item.address && item.verified)
            .count()
            == 0;

        require!(
            !collection_not_proper && metadata.mint == ctx.accounts.mint.key(),
            RaffleError::InvalidNft
        );

        require!(nft_count >= a_pool.min_nft_count, RaffleError::InsufficientNft);

        require!(amount > 0, RaffleError::InvalidAmount);
        if total_ticket != MAX_TOTAL_TICKET  {
            require!(a_pool.purchased_ticket + amount <= a_pool.total_ticket, 
            RaffleError::TooManyTicket);
        }
        require!(
            current_time >= a_pool.start_time && 
            current_time <= a_pool.end_time, 
            RaffleError::OutOfRaffle
        );
        
        token::transfer(ctx.accounts.transfer_context(), a_pool.ticket_price * amount as u64)?;

        a_pool.buy_ticket(a_buyer.to_account_info().key(), amount)?;
        Ok(())
    }

    pub fn set_winner(ctx: Context<SetWinnerContext>) -> Result<()> {
        let mut a_pool = ctx.accounts.pool.load_mut()?;
        let a_slothash = &ctx.accounts.slothash;
        // msg!("a_slothash: {}", a_slothash);

        let current_time = get_current_time()?;
        require!(
            current_time >= a_pool.end_time, 
            RaffleError::NotFinishRaffle
        );

        require!(a_pool.purchased_ticket > 0, RaffleError::SetWinnerError);
        require!(a_pool.state == 0, RaffleError::AlreadySetWinner);
        
        let random = u64::from_le_bytes(
            a_slothash.to_account_info().data.borrow()[16..24].try_into().unwrap()
        );
        
        a_pool.set_winner(random)?;
        a_pool.state = 1;

        Ok(())
    }

    pub fn claim_prize(ctx: Context<ClaimPrizeContext>) -> Result<()> {
        {
            let mut a_pool = ctx.accounts.pool.load_mut()?;

            let a_buyer = &ctx.accounts.buyer;
    
            let current_time = get_current_time()?;
    
            require!(
                current_time >= a_pool.end_time, 
                RaffleError::NotFinishRaffle
            );
    
            require!(
                a_pool.state == 1, 
                RaffleError::ClaimPrizeError
            );
    
            let result = a_pool.claim_prize(a_buyer.to_account_info().key())?;
            require!(result, RaffleError::NotWinner);
            a_pool.state = 2; 
        }

        {
            let a_pool = ctx.accounts.pool.load()?;
            let (_pool, bump) = Pubkey::find_program_address(
                &[POOL_SEED.as_ref(), 
                &a_pool.raffle_id.to_le_bytes(), 
                a_pool.mint.key().as_ref()], 
                ctx.program_id
            );
            
            let seeds = &[POOL_SEED.as_bytes(), &a_pool.raffle_id.to_le_bytes(), a_pool.mint.as_ref(), &[bump]];
            let signer = &[&seeds[..]];
    
            token::transfer(
                ctx.accounts.transfer_context().with_signer(signer), 
                1
            )?;
    
            Ok(())
        }
    }

    pub fn send_back_nft(ctx: Context<SendBackNftContext>) -> Result<()> {
        {
            let mut a_pool = ctx.accounts.pool.load_mut()?;
    
            let current_time = get_current_time()?;
    
            require!(
                current_time >= a_pool.end_time, 
                RaffleError::NotFinishRaffle
            );
    
            require!(
                a_pool.state == 0, 
                RaffleError::SendBackNftError
            );
    
            a_pool.state = 3; 
        }

        {
            let a_pool = ctx.accounts.pool.load()?;
            let (_pool, bump) = Pubkey::find_program_address(
                &[POOL_SEED.as_ref(), 
                &a_pool.raffle_id.to_le_bytes(), 
                a_pool.mint.key().as_ref()], 
                ctx.program_id
            );
            
            let seeds = &[POOL_SEED.as_bytes(), &a_pool.raffle_id.to_le_bytes(), a_pool.mint.as_ref(), &[bump]];
            let signer = &[&seeds[..]];
    
            token::transfer(
                ctx.accounts.transfer_context().with_signer(signer), 
                1
            )?;
    
            Ok(())
        }
    }
}
