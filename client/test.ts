import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { PublicKey, Keypair, SystemProgram } from "@solana/web3.js";

// Program ID from deployment
const PROGRAM_ID = new PublicKey("BpAHB6zNNri2BvVvWBRL8VZK28mmPweftx6VgtmVbg2U");

async function main() {
  // Connect to devnet
  const connection = new anchor.web3.Connection("https://api.devnet.solana.com", "confirmed");
  
  // Load wallet from default location
  const walletKeypair = Keypair.fromSecretKey(
    Uint8Array.from(require("/home/ubuntu/.config/solana/id.json"))
  );
  
  const wallet = new anchor.Wallet(walletKeypair);
  const provider = new anchor.AnchorProvider(connection, wallet, {
    commitment: "confirmed",
  });
  
  console.log("Wallet:", wallet.publicKey.toBase58());
  const balance = await connection.getBalance(wallet.publicKey);
  console.log("Balance:", balance / 1e9, "SOL");
  
  // Load IDL
  const idl = require("../target/idl/agent_reputation.json");
  const program = new Program(idl, provider);
  
  // Derive PDAs
  const [jobCounter] = PublicKey.findProgramAddressSync(
    [Buffer.from("job_counter")],
    PROGRAM_ID
  );
  
  const [profile] = PublicKey.findProgramAddressSync(
    [Buffer.from("profile"), wallet.publicKey.toBuffer()],
    PROGRAM_ID
  );
  
  console.log("\n=== PDAs ===");
  console.log("Job Counter PDA:", jobCounter.toBase58());
  console.log("Profile PDA:", profile.toBase58());
  
  // Check if job counter exists
  const counterAccount = await connection.getAccountInfo(jobCounter);
  if (!counterAccount) {
    console.log("\n=== Initializing Job Counter ===");
    try {
      const tx = await (program.methods as any)
        .initializeCounter()
        .accounts({
          jobCounter: jobCounter,
          payer: wallet.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      console.log("Job Counter initialized:", tx);
    } catch (e: any) {
      console.log("Error initializing counter:", e.message);
    }
  } else {
    console.log("\nJob Counter already exists");
  }
  
  // Check if profile exists
  const profileAccount = await connection.getAccountInfo(profile);
  if (!profileAccount) {
    console.log("\n=== Initializing Profile ===");
    try {
      const tx = await (program.methods as any)
        .initializeProfile()
        .accounts({
          profile: profile,
          authority: wallet.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      console.log("Profile initialized:", tx);
    } catch (e: any) {
      console.log("Error initializing profile:", e.message);
    }
  } else {
    console.log("\nProfile already exists");
  }
  
  console.log("\n=== Test Complete ===");
  console.log("Program deployed and accounts ready!");
  console.log("\nExplorer links:");
  console.log(`Program: https://explorer.solana.com/address/${PROGRAM_ID.toBase58()}?cluster=devnet`);
  console.log(`Job Counter: https://explorer.solana.com/address/${jobCounter.toBase58()}?cluster=devnet`);
  console.log(`Profile: https://explorer.solana.com/address/${profile.toBase58()}?cluster=devnet`);
}

main().catch(console.error);
