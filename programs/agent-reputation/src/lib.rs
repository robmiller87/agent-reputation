use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("BpAHB6zNNri2BvVvWBRL8VZK28mmPweftx6VgtmVbg2U");

#[program]
pub mod agent_reputation {
    use super::*;

    /// Initialize the job counter (one-time setup)
    pub fn initialize_counter(ctx: Context<InitializeCounter>) -> Result<()> {
        let counter = &mut ctx.accounts.job_counter;
        counter.count = 0;
        counter.bump = ctx.bumps.job_counter;
        Ok(())
    }

    /// Initialize a new agent profile
    pub fn initialize_profile(ctx: Context<InitializeProfile>) -> Result<()> {
        let profile = &mut ctx.accounts.profile;
        profile.authority = ctx.accounts.authority.key();
        profile.total_jobs = 0;
        profile.total_rating_points = 0;
        profile.completed_jobs = 0;
        profile.on_time_count = 0;
        profile.disputed_count = 0;
        profile.created_at = Clock::get()?.unix_timestamp;
        profile.bump = ctx.bumps.profile;
        Ok(())
    }

    /// Create a new job with SOL escrow
    pub fn create_job(
        ctx: Context<CreateJob>,
        description_hash: [u8; 32],
        payment: u64,
        deadline: i64,
        assigned_worker: Option<Pubkey>, // Optional: whitelist specific worker
    ) -> Result<()> {
        require!(payment >= 1_000_000, ErrorCode::PaymentTooSmall); // 0.001 SOL min
        require!(deadline > Clock::get()?.unix_timestamp, ErrorCode::InvalidDeadline);
        require!(deadline < Clock::get()?.unix_timestamp + 30 * 24 * 60 * 60, ErrorCode::DeadlineTooFar); // Max 30 days
        
        let job = &mut ctx.accounts.job;
        let counter = &mut ctx.accounts.job_counter;
        
        job.id = counter.count;
        job.client = ctx.accounts.client.key();
        job.worker = Pubkey::default();
        job.assigned_worker = assigned_worker;
        job.description_hash = description_hash;
        job.payment = payment;
        job.deadline = deadline;
        job.status = JobStatus::Open;
        job.created_at = Clock::get()?.unix_timestamp;
        job.bump = ctx.bumps.job;

        // Transfer SOL to escrow (job PDA)
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.client.to_account_info(),
                    to: ctx.accounts.job.to_account_info(),
                },
            ),
            payment,
        )?;

        // Increment job counter
        counter.count += 1;

        Ok(())
    }

    /// Accept a job as a worker
    pub fn accept_job(ctx: Context<AcceptJob>) -> Result<()> {
        let job = &mut ctx.accounts.job;
        require!(job.status == JobStatus::Open, ErrorCode::JobNotOpen);
        require!(Clock::get()?.unix_timestamp < job.deadline, ErrorCode::JobExpired);
        
        // Check worker whitelist if set
        if job.assigned_worker != Some(Pubkey::default()) && job.assigned_worker.is_some() {
            require!(
                job.assigned_worker == Some(ctx.accounts.worker.key()),
                ErrorCode::WorkerNotWhitelisted
            );
        }
        
        job.worker = ctx.accounts.worker.key();
        job.status = JobStatus::Accepted;
        
        Ok(())
    }

    /// Complete a job and release payment + submit rating
    pub fn complete_job(
        ctx: Context<CompleteJob>,
        rating: u8,
        on_time: bool,
        comment_hash: [u8; 32],
    ) -> Result<()> {
        require!(rating >= 1 && rating <= 5, ErrorCode::InvalidRating);
        
        let job = &mut ctx.accounts.job;
        require!(job.status == JobStatus::Accepted, ErrorCode::JobNotAccepted);
        require!(job.client == ctx.accounts.client.key(), ErrorCode::NotJobClient);
        
        let payment_amount = job.payment;
        job.status = JobStatus::Completed;

        // Transfer SOL from escrow to worker
        let job_account_info = job.to_account_info();
        let worker_account_info = ctx.accounts.worker.to_account_info();
        
        **job_account_info.try_borrow_mut_lamports()? -= payment_amount;
        **worker_account_info.try_borrow_mut_lamports()? += payment_amount;

        // Update worker's profile
        let worker_profile = &mut ctx.accounts.worker_profile;
        worker_profile.total_jobs += 1;
        worker_profile.completed_jobs += 1;
        // Use u64 for precision: (rating * 100) to store as percentage points
        worker_profile.total_rating_points += (rating as u32) * 100;
        if on_time {
            worker_profile.on_time_count += 1;
        }

        // Create rating record
        let rating_record = &mut ctx.accounts.rating;
        rating_record.job_id = job.id;
        rating_record.client = ctx.accounts.client.key();
        rating_record.worker = job.worker;
        rating_record.score = rating;
        rating_record.on_time = on_time;
        rating_record.comment_hash = comment_hash;
        rating_record.timestamp = Clock::get()?.unix_timestamp;
        rating_record.bump = ctx.bumps.rating;

        Ok(())
    }

    /// Cancel a job (only if still open, refund to client)
    pub fn cancel_job(ctx: Context<CancelJob>) -> Result<()> {
        let job = &mut ctx.accounts.job;
        require!(job.status == JobStatus::Open, ErrorCode::CannotCancel);
        require!(job.client == ctx.accounts.client.key(), ErrorCode::NotJobClient);
        
        let refund_amount = job.payment;
        job.status = JobStatus::Cancelled;

        // Refund SOL to client
        let job_account_info = job.to_account_info();
        let client_account_info = ctx.accounts.client.to_account_info();
        
        **job_account_info.try_borrow_mut_lamports()? -= refund_amount;
        **client_account_info.try_borrow_mut_lamports()? += refund_amount;

        Ok(())
    }

    /// Dispute a job (either party can call)
    pub fn dispute_job(ctx: Context<DisputeJob>) -> Result<()> {
        let job = &mut ctx.accounts.job;
        require!(job.status == JobStatus::Accepted, ErrorCode::CannotDispute);
        require!(
            ctx.accounts.disputer.key() == job.client || ctx.accounts.disputer.key() == job.worker,
            ErrorCode::NotPartyToJob
        );
        
        job.status = JobStatus::Disputed;
        job.dispute_timestamp = Some(Clock::get()?.unix_timestamp);
        
        Ok(())
    }

    /// Resolve dispute after timeout (7 days) - splits payment 50/50
    pub fn resolve_dispute(ctx: Context<ResolveDispute>) -> Result<()> {
        let job = &mut ctx.accounts.job;
        require!(job.status == JobStatus::Disputed, ErrorCode::NotDisputed);
        
        let dispute_time = job.dispute_timestamp.ok_or(ErrorCode::NotDisputed)?;
        let resolution_time = dispute_time + (7 * 24 * 60 * 60); // 7 days
        require!(
            Clock::get()?.unix_timestamp >= resolution_time,
            ErrorCode::ResolutionPeriodNotEnded
        );
        
        let payment_amount = job.payment;
        let half_payment = payment_amount / 2;
        job.status = JobStatus::Resolved;

        // Split payment 50/50 between client and worker
        let job_account_info = job.to_account_info();
        let client_account_info = ctx.accounts.client.to_account_info();
        let worker_account_info = ctx.accounts.worker.to_account_info();
        
        **job_account_info.try_borrow_mut_lamports()? -= payment_amount;
        **client_account_info.try_borrow_mut_lamports()? += half_payment;
        **worker_account_info.try_borrow_mut_lamports()? += payment_amount - half_payment; // Handle odd amounts

        // Update worker's disputed count
        let worker_profile = &mut ctx.accounts.worker_profile;
        worker_profile.disputed_count += 1;

        Ok(())
    }

    /// Auto-refund expired jobs (deadline passed, no worker accepted)
    pub fn refund_expired(ctx: Context<RefundExpired>) -> Result<()> {
        let job = &mut ctx.accounts.job;
        require!(job.status == JobStatus::Open, ErrorCode::CannotRefund);
        require!(Clock::get()?.unix_timestamp > job.deadline, ErrorCode::JobNotExpired);
        
        let refund_amount = job.payment;
        job.status = JobStatus::Cancelled;

        // Refund SOL to client
        let job_account_info = job.to_account_info();
        let client_account_info = ctx.accounts.client.to_account_info();
        
        **job_account_info.try_borrow_mut_lamports()? -= refund_amount;
        **client_account_info.try_borrow_mut_lamports()? += refund_amount;

        Ok(())
    }

    /// Get reputation (view function - returns profile data with proper precision)
    pub fn get_reputation(ctx: Context<GetReputation>) -> Result<()> {
        let profile = &ctx.accounts.profile;
        msg!("Agent: {}", profile.authority);
        msg!("Total Jobs: {}", profile.total_jobs);
        msg!("Completed: {}", profile.completed_jobs);
        msg!("Disputed: {}", profile.disputed_count);
        msg!("On-Time: {}", profile.on_time_count);
        if profile.completed_jobs > 0 {
            // Rating stored as percentage points (e.g., 450 = 4.50/5)
            let avg_rating_x100 = profile.total_rating_points / profile.completed_jobs;
            let on_time_pct = (profile.on_time_count as u64 * 10000) / (profile.completed_jobs as u64);
            msg!("Avg Rating: {}.{}/5", avg_rating_x100 / 100, avg_rating_x100 % 100);
            msg!("On-Time Rate: {}.{}%", on_time_pct / 100, on_time_pct % 100);
        }
        Ok(())
    }
}

// ============ Account Structs ============

#[derive(Accounts)]
pub struct InitializeCounter<'info> {
    #[account(
        init,
        payer = payer,
        space = 8 + JobCounter::INIT_SPACE,
        seeds = [b"job_counter"],
        bump
    )]
    pub job_counter: Account<'info, JobCounter>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct InitializeProfile<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + AgentProfile::INIT_SPACE,
        seeds = [b"profile", authority.key().as_ref()],
        bump
    )]
    pub profile: Account<'info, AgentProfile>,
    #[account(mut)]
    pub authority: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CreateJob<'info> {
    #[account(
        init,
        payer = client,
        space = 8 + Job::INIT_SPACE,
        seeds = [b"job", job_counter.count.to_le_bytes().as_ref()],
        bump
    )]
    pub job: Account<'info, Job>,
    #[account(
        mut,
        seeds = [b"job_counter"],
        bump = job_counter.bump
    )]
    pub job_counter: Account<'info, JobCounter>,
    #[account(mut)]
    pub client: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct AcceptJob<'info> {
    #[account(mut)]
    pub job: Account<'info, Job>,
    pub worker: Signer<'info>,
}

#[derive(Accounts)]
pub struct CompleteJob<'info> {
    #[account(mut)]
    pub job: Account<'info, Job>,
    #[account(
        init,
        payer = client,
        space = 8 + Rating::INIT_SPACE,
        seeds = [b"rating", job.key().as_ref()],
        bump
    )]
    pub rating: Account<'info, Rating>,
    #[account(
        mut,
        seeds = [b"profile", job.worker.as_ref()],
        bump = worker_profile.bump
    )]
    pub worker_profile: Account<'info, AgentProfile>,
    /// CHECK: Worker receives payment, validated by job.worker
    #[account(mut, constraint = worker.key() == job.worker)]
    pub worker: AccountInfo<'info>,
    #[account(mut)]
    pub client: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CancelJob<'info> {
    #[account(mut)]
    pub job: Account<'info, Job>,
    #[account(mut)]
    pub client: Signer<'info>,
}

#[derive(Accounts)]
pub struct DisputeJob<'info> {
    #[account(mut)]
    pub job: Account<'info, Job>,
    pub disputer: Signer<'info>,
}

#[derive(Accounts)]
pub struct ResolveDispute<'info> {
    #[account(mut)]
    pub job: Account<'info, Job>,
    #[account(
        mut,
        seeds = [b"profile", job.worker.as_ref()],
        bump = worker_profile.bump
    )]
    pub worker_profile: Account<'info, AgentProfile>,
    /// CHECK: Client receives refund, validated by job.client
    #[account(mut, constraint = client.key() == job.client)]
    pub client: AccountInfo<'info>,
    /// CHECK: Worker receives partial payment, validated by job.worker
    #[account(mut, constraint = worker.key() == job.worker)]
    pub worker: AccountInfo<'info>,
    pub resolver: Signer<'info>, // Anyone can call after timeout
}

#[derive(Accounts)]
pub struct RefundExpired<'info> {
    #[account(mut)]
    pub job: Account<'info, Job>,
    /// CHECK: Client receives refund, validated by job.client
    #[account(mut, constraint = client.key() == job.client)]
    pub client: AccountInfo<'info>,
    pub caller: Signer<'info>, // Anyone can call after expiry
}

#[derive(Accounts)]
pub struct GetReputation<'info> {
    pub profile: Account<'info, AgentProfile>,
}

// ============ Data Structs ============

#[account]
#[derive(InitSpace)]
pub struct AgentProfile {
    pub authority: Pubkey,
    pub total_jobs: u32,
    pub completed_jobs: u32,
    pub total_rating_points: u32, // Stored as score * 100 for precision
    pub on_time_count: u32,
    pub disputed_count: u32,
    pub created_at: i64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct Job {
    pub id: u64,
    pub client: Pubkey,
    pub worker: Pubkey,
    pub assigned_worker: Option<Pubkey>, // Optional whitelist
    pub description_hash: [u8; 32],
    pub payment: u64,
    pub deadline: i64,
    pub status: JobStatus,
    pub created_at: i64,
    pub dispute_timestamp: Option<i64>,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct JobCounter {
    pub count: u64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct Rating {
    pub job_id: u64,
    pub client: Pubkey,
    pub worker: Pubkey,
    pub score: u8,
    pub on_time: bool,
    pub comment_hash: [u8; 32],
    pub timestamp: i64,
    pub bump: u8,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq, InitSpace)]
pub enum JobStatus {
    Open,
    Accepted,
    Completed,
    Disputed,
    Resolved,
    Cancelled,
}

impl Default for JobStatus {
    fn default() -> Self {
        JobStatus::Open
    }
}

// ============ Errors ============

#[error_code]
pub enum ErrorCode {
    #[msg("Job is not open for acceptance")]
    JobNotOpen,
    #[msg("Job has not been accepted yet")]
    JobNotAccepted,
    #[msg("Only the job client can complete the job")]
    NotJobClient,
    #[msg("Rating must be between 1 and 5")]
    InvalidRating,
    #[msg("Payment must be at least 0.001 SOL")]
    PaymentTooSmall,
    #[msg("Deadline must be in the future")]
    InvalidDeadline,
    #[msg("Deadline cannot be more than 30 days away")]
    DeadlineTooFar,
    #[msg("Job has expired")]
    JobExpired,
    #[msg("Worker not whitelisted for this job")]
    WorkerNotWhitelisted,
    #[msg("Job cannot be cancelled in current state")]
    CannotCancel,
    #[msg("Job cannot be disputed in current state")]
    CannotDispute,
    #[msg("Not a party to this job")]
    NotPartyToJob,
    #[msg("Job is not in disputed state")]
    NotDisputed,
    #[msg("Resolution period has not ended (7 days)")]
    ResolutionPeriodNotEnded,
    #[msg("Job has not expired yet")]
    JobNotExpired,
    #[msg("Cannot refund job in current state")]
    CannotRefund,
}
