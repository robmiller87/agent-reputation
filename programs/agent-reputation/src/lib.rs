use anchor_lang::prelude::*;

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
        profile.created_at = Clock::get()?.unix_timestamp;
        profile.bump = ctx.bumps.profile;
        Ok(())
    }

    /// Create a new job
    pub fn create_job(
        ctx: Context<CreateJob>,
        description_hash: [u8; 32],
        payment: u64,
        deadline: i64,
    ) -> Result<()> {
        let job = &mut ctx.accounts.job;
        let counter = &mut ctx.accounts.job_counter;
        
        job.id = counter.count;
        job.client = ctx.accounts.client.key();
        job.worker = Pubkey::default();
        job.description_hash = description_hash;
        job.payment = payment;
        job.deadline = deadline;
        job.status = JobStatus::Open;
        job.created_at = Clock::get()?.unix_timestamp;
        job.bump = ctx.bumps.job;

        // Increment job counter
        counter.count += 1;

        Ok(())
    }

    /// Accept a job as a worker
    pub fn accept_job(ctx: Context<AcceptJob>) -> Result<()> {
        let job = &mut ctx.accounts.job;
        require!(job.status == JobStatus::Open, ErrorCode::JobNotOpen);
        
        job.worker = ctx.accounts.worker.key();
        job.status = JobStatus::Accepted;
        
        Ok(())
    }

    /// Complete a job and submit rating
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
        
        job.status = JobStatus::Completed;

        // Update worker's profile
        let worker_profile = &mut ctx.accounts.worker_profile;
        worker_profile.total_jobs += 1;
        worker_profile.completed_jobs += 1;
        worker_profile.total_rating_points += rating as u32;
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

    /// Get reputation (view function - just returns profile data)
    pub fn get_reputation(ctx: Context<GetReputation>) -> Result<()> {
        let profile = &ctx.accounts.profile;
        msg!("Agent: {}", profile.authority);
        msg!("Total Jobs: {}", profile.total_jobs);
        msg!("Completed: {}", profile.completed_jobs);
        msg!("Rating Points: {}", profile.total_rating_points);
        msg!("On-Time: {}", profile.on_time_count);
        if profile.completed_jobs > 0 {
            let avg_rating = profile.total_rating_points / profile.completed_jobs;
            let on_time_pct = (profile.on_time_count * 100) / profile.completed_jobs;
            msg!("Avg Rating: {}/5", avg_rating);
            msg!("On-Time Rate: {}%", on_time_pct);
        }
        Ok(())
    }
}

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
    #[account(mut)]
    pub client: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct GetReputation<'info> {
    pub profile: Account<'info, AgentProfile>,
}

#[account]
#[derive(InitSpace)]
pub struct AgentProfile {
    pub authority: Pubkey,
    pub total_jobs: u32,
    pub completed_jobs: u32,
    pub total_rating_points: u32,
    pub on_time_count: u32,
    pub created_at: i64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct Job {
    pub id: u64,
    pub client: Pubkey,
    pub worker: Pubkey,
    pub description_hash: [u8; 32],
    pub payment: u64,
    pub deadline: i64,
    pub status: JobStatus,
    pub created_at: i64,
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
    Cancelled,
}

impl Default for JobStatus {
    fn default() -> Self {
        JobStatus::Open
    }
}

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
}
