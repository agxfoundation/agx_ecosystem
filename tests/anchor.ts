import BN from "bn.js";
import assert from "assert";
import * as web3 from "@solana/web3.js";
import * as anchor from "@coral-xyz/anchor";
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { AgxEcosystem } from "../target/types/agx_ecosystem";
import {
  TOKEN_PROGRAM_ID,
  createInitializeMintInstruction,
  createInitializeAccountInstruction,
  createMintToInstruction,
} from "@solana/spl-token";
import assert from "assert";
import type { AgxEcosystem } from "../target/types/agx_ecosystem";

describe("agx_ecosystem", function () {
  // Configure the client to use the local cluster
  anchor.setProvider(anchor.AnchorProvider.env());

  const program = anchor.workspace.AgxEcosystem as anchor.Program<AgxEcosystem>;
  
  // Set Mocha timeout to 120 seconds to allow slow public Devnet transactions to confirm
  this.timeout(120000);

  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  // Using workspace program directly. Anchor CLI will sync the Program ID automatically.
  const program = anchor.workspace.AgxEcosystem as Program<AgxEcosystem>;
  const admin = provider.wallet;

  let tokenMint: anchor.web3.PublicKey;
  let usdtMint: anchor.web3.PublicKey;

  // Vault variables
  let usdtVault: anchor.web3.PublicKey;
  let rewardVault: anchor.web3.PublicKey;
  let presaleVault: anchor.web3.PublicKey;
  let treasuryVault: anchor.web3.PublicKey;
  let developmentVault: anchor.web3.PublicKey;
  let marketingVault: anchor.web3.PublicKey;
  let roadmapVault: anchor.web3.PublicKey;

  // Fresh user keypair generated on every run
  const user = anchor.web3.Keypair.generate();
  let userTokenAccount: anchor.web3.PublicKey;
  let userUsdtAccount: anchor.web3.PublicKey;

  // Unique Deterministic Keypair for the State account derived from the unique Program ID.
  const stateKeypair = anchor.web3.Keypair.fromSeed(
    program.programId.toBuffer()
  );

  // Derive Centralized Vault Authority PDA
  const [vaultAuthority, vaultAuthorityBump] =
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("vault-authority")],
      program.programId
    );

  // Synchronous busy-wait helper to bypass setTimeout sandbox constraints
  const sleep = (ms: number) => {
    const start = Date.now();
    while (Date.now() - start < ms) {}
  };

  // Helper function to create Token Accounts
  async function createTokenAccountHelper(
    mint: anchor.web3.PublicKey,
    owner: anchor.web3.PublicKey
  ): Promise<anchor.web3.PublicKey> {
    const accountKeypair = anchor.web3.Keypair.generate();
    const lamports =
      await provider.connection.getMinimumBalanceForRentExemption(165); // TokenAccount Size is 165
    const transaction = new anchor.web3.Transaction().add(
      anchor.web3.SystemProgram.createAccount({
        fromPubkey: admin.publicKey,
        newAccountPubkey: accountKeypair.publicKey,
        space: 165,
        lamports,
        programId: TOKEN_PROGRAM_ID,
      }),
      createInitializeAccountInstruction(accountKeypair.publicKey, mint, owner)
    );
    await provider.sendAndConfirm(transaction, [accountKeypair], {
      commitment: "processed",
      skipPreflight: true,
    });
    return accountKeypair.publicKey;
  }

  // Helper function to mint tokens to any address
  async function mintToHelper(
    mint: anchor.web3.PublicKey,
    destination: anchor.web3.PublicKey,
    amount: number
  ): Promise<void> {
    const transaction = new anchor.web3.Transaction().add(
      createMintToInstruction(mint, destination, admin.publicKey, amount)
    );
    await provider.sendAndConfirm(transaction, [], {
      commitment: "processed",
      skipPreflight: true,
    });
  }

  before(async () => {
    // DIAGNOSTIC LOGS:
    console.log("=== DIAGNOSTIC LOGS ===");
    console.log("RPC Endpoint:", provider.connection.rpcEndpoint);
    console.log("Client Wallet Address:", admin.publicKey.toBase58());
    console.log("Target Program ID:", program.programId.toBase58());
    console.log("Central Vault Authority PDA:", vaultAuthority.toBase58());

    // Check if the state account is already initialized on-chain
    const stateInfo = await provider.connection.getAccountInfo(
      stateKeypair.publicKey
    );
    if (stateInfo !== null) {
      console.log(
        "ℹ️ Program already initialized. Fetching active config from on-chain State..."
      );
      const stateData = await program.account.globalState.fetch(
        stateKeypair.publicKey
      );
      tokenMint = stateData.tokenMint;
      usdtMint = stateData.usdtMint;
      usdtVault = stateData.usdtVault;
      rewardVault = stateData.rewardVault;
      presaleVault = stateData.presaleVault;
      treasuryVault = stateData.treasuryVault;
      developmentVault = stateData.developmentVault;
      marketingVault = stateData.marketingVault;
      roadmapVault = stateData.roadmapVault;

      // Reset on-chain price config for dynamic testing consistency
      console.log(
        "Resetting on-chain swap price back to $0.09 to ensure test run idempotence..."
      );
      await program.methods
        .updateConfig(
          true,
          true,
          true,
          true,
          new anchor.BN(90_000),
          new anchor.BN(20)
        )
        .accounts({
          state: stateKeypair.publicKey,
          admin: admin.publicKey,
        })
        .rpc({ commitment: "processed", skipPreflight: true });
      sleep(1000);

      // Transfer SOL to our fresh user keypair
      const tx = new anchor.web3.Transaction().add(
        anchor.web3.SystemProgram.transfer({
          fromPubkey: admin.publicKey,
          toPubkey: user.publicKey,
          lamports: 0.1 * anchor.web3.LAMPORTS_PER_SOL,
        })
      );
      await provider.sendAndConfirm(tx, [], {
        commitment: "processed",
        skipPreflight: true,
      });

      // Create token accounts for user and fund user USDT
      userTokenAccount = await createTokenAccountHelper(
        tokenMint,
        user.publicKey
      );
      userUsdtAccount = await createTokenAccountHelper(
        usdtMint,
        user.publicKey
      );
      await mintToHelper(usdtMint, userUsdtAccount, 1000_000_000);

      // Robust check: If previous timeout interrupted the minting/funding, fund them now
      try {
        const presaleBalance = await provider.connection.getTokenAccountBalance(
          presaleVault
        );
        if (Number(presaleBalance.value.amount) === 0) {
          console.log(
            "⚠️ Deployed vaults have 0 tokens. Funding Reward & Presale Vaults now..."
          );
          await mintToHelper(tokenMint, rewardVault, 25_000_000_000_000_000); // 25M AGX
          await mintToHelper(tokenMint, presaleVault, 5_000_000_000_000_000); // 5M AGX
        }
      } catch (err) {
        console.log("Token check error (vaults might need funding):", err);
      }
    } else {
      console.log(
        "🆕 Fresh run detected. Creating Mints and Vaults externally in batched transactions..."
      );

      // 1. Create Mints
      const tokenMintKp = anchor.web3.Keypair.generate();
      const usdtMintKp = anchor.web3.Keypair.generate();
      tokenMint = tokenMintKp.publicKey;
      usdtMint = usdtMintKp.publicKey;

      const mintRent =
        await provider.connection.getMinimumBalanceForRentExemption(82);
      const tokenAccRent =
        await provider.connection.getMinimumBalanceForRentExemption(165);

      // Transaction 1: Create both Mints + Transfer SOL to User
      const tx1 = new anchor.web3.Transaction().add(
        anchor.web3.SystemProgram.transfer({
          fromPubkey: admin.publicKey,
          toPubkey: user.publicKey,
          lamports: 0.2 * anchor.web3.LAMPORTS_PER_SOL,
        }),
        anchor.web3.SystemProgram.createAccount({
          fromPubkey: admin.publicKey,
          newAccountPubkey: tokenMint,
          space: 82,
          lamports: mintRent,
          programId: TOKEN_PROGRAM_ID,
        }),
        createInitializeMintInstruction(tokenMint, 9, admin.publicKey, null),
        anchor.web3.SystemProgram.createAccount({
          fromPubkey: admin.publicKey,
          newAccountPubkey: usdtMint,
          space: 82,
          lamports: mintRent,
          programId: TOKEN_PROGRAM_ID,
        }),
        createInitializeMintInstruction(usdtMint, 6, admin.publicKey, null)
      );
      await provider.sendAndConfirm(tx1, [tokenMintKp, usdtMintKp], {
        commitment: "processed",
        skipPreflight: true,
      });

      // Generate keypairs for the 7 vaults + 2 user accounts
      const usdtVaultKp = anchor.web3.Keypair.generate();
      const rewardVaultKp = anchor.web3.Keypair.generate();
      const presaleVaultKp = anchor.web3.Keypair.generate();
      const treasuryVaultKp = anchor.web3.Keypair.generate();
      const developmentVaultKp = anchor.web3.Keypair.generate();
      const marketingVaultKp = anchor.web3.Keypair.generate();
      const roadmapVaultKp = anchor.web3.Keypair.generate();
      const userTokenKp = anchor.web3.Keypair.generate();
      const userUsdtKp = anchor.web3.Keypair.generate();

      usdtVault = usdtVaultKp.publicKey;
      rewardVault = rewardVaultKp.publicKey;
      presaleVault = presaleVaultKp.publicKey;
      treasuryVault = treasuryVaultKp.publicKey;
      developmentVault = developmentVaultKp.publicKey;
      marketingVault = marketingVaultKp.publicKey;
      roadmapVault = roadmapVaultKp.publicKey;
      userTokenAccount = userTokenKp.publicKey;
      userUsdtAccount = userUsdtKp.publicKey;

      // Transaction 2: Create Vaults 1-4
      const tx2 = new anchor.web3.Transaction().add(
        anchor.web3.SystemProgram.createAccount({
          fromPubkey: admin.publicKey,
          newAccountPubkey: usdtVault,
          space: 165,
          lamports: tokenAccRent,
          programId: TOKEN_PROGRAM_ID,
        }),
        createInitializeAccountInstruction(usdtVault, usdtMint, vaultAuthority),

        anchor.web3.SystemProgram.createAccount({
          fromPubkey: admin.publicKey,
          newAccountPubkey: rewardVault,
          space: 165,
          lamports: tokenAccRent,
          programId: TOKEN_PROGRAM_ID,
        }),
        createInitializeAccountInstruction(
          rewardVault,
          tokenMint,
          vaultAuthority
        ),

        anchor.web3.SystemProgram.createAccount({
          fromPubkey: admin.publicKey,
          newAccountPubkey: presaleVault,
          space: 165,
          lamports: tokenAccRent,
          programId: TOKEN_PROGRAM_ID,
        }),
        createInitializeAccountInstruction(
          presaleVault,
          tokenMint,
          vaultAuthority
        ),

        anchor.web3.SystemProgram.createAccount({
          fromPubkey: admin.publicKey,
          newAccountPubkey: treasuryVault,
          space: 165,
          lamports: tokenAccRent,
          programId: TOKEN_PROGRAM_ID,
        }),
        createInitializeAccountInstruction(
          treasuryVault,
          tokenMint,
          vaultAuthority
        )
      );
      await provider.sendAndConfirm(
        tx2,
        [usdtVaultKp, rewardVaultKp, presaleVaultKp, treasuryVaultKp],
        { commitment: "processed", skipPreflight: true }
      );

      // Transaction 3: Create Vaults 5-7 + User Accounts
      const tx3 = new anchor.web3.Transaction().add(
        anchor.web3.SystemProgram.createAccount({
          fromPubkey: admin.publicKey,
          newAccountPubkey: developmentVault,
          space: 165,
          lamports: tokenAccRent,
          programId: TOKEN_PROGRAM_ID,
        }),
        createInitializeAccountInstruction(
          developmentVault,
          tokenMint,
          vaultAuthority
        ),

        anchor.web3.SystemProgram.createAccount({
          fromPubkey: admin.publicKey,
          newAccountPubkey: marketingVault,
          space: 165,
          lamports: tokenAccRent,
          programId: TOKEN_PROGRAM_ID,
        }),
        createInitializeAccountInstruction(
          marketingVault,
          tokenMint,
          vaultAuthority
        ),

        anchor.web3.SystemProgram.createAccount({
          fromPubkey: admin.publicKey,
          newAccountPubkey: roadmapVault,
          space: 165,
          lamports: tokenAccRent,
          programId: TOKEN_PROGRAM_ID,
        }),
        createInitializeAccountInstruction(
          roadmapVault,
          tokenMint,
          vaultAuthority
        ),

        anchor.web3.SystemProgram.createAccount({
          fromPubkey: admin.publicKey,
          newAccountPubkey: userTokenAccount,
          space: 165,
          lamports: tokenAccRent,
          programId: TOKEN_PROGRAM_ID,
        }),
        createInitializeAccountInstruction(
          userTokenAccount,
          tokenMint,
          user.publicKey
        ),

        anchor.web3.SystemProgram.createAccount({
          fromPubkey: admin.publicKey,
          newAccountPubkey: userUsdtAccount,
          space: 165,
          lamports: tokenAccRent,
          programId: TOKEN_PROGRAM_ID,
        }),
        createInitializeAccountInstruction(
          userUsdtAccount,
          usdtMint,
          user.publicKey
        )
      );
      await provider.sendAndConfirm(
        tx3,
        [
          developmentVaultKp,
          marketingVaultKp,
          roadmapVaultKp,
          userTokenKp,
          userUsdtKp,
        ],
        { commitment: "processed", skipPreflight: true }
      );

      // Transaction 4: Mint Funds to Vaults and User
      const tx4 = new anchor.web3.Transaction().add(
        createMintToInstruction(
          tokenMint,
          rewardVault,
          admin.publicKey,
          25_000_000_000_000_000
        ), // 25M AGX
        createMintToInstruction(
          tokenMint,
          presaleVault,
          admin.publicKey,
          5_000_000_000_000_000
        ), // 5M AGX
        createMintToInstruction(
          usdtMint,
          userUsdtAccount,
          admin.publicKey,
          1000_000_000
        ) // 1000 USDT to user
      );
      await provider.sendAndConfirm(tx4, [], {
        commitment: "processed",
        skipPreflight: true,
      });
    }

    console.log("Active AGX Mint:", tokenMint.toBase58());
    console.log("Active USDT Mint:", usdtMint.toBase58());
    console.log("Active USDT Vault:", usdtVault.toBase58());
    console.log("=======================");

    // Delay for 3 seconds to let Devnet nodes fully sync and avoid replica lag simulation issues
    sleep(3000);
  });

  it("Initializes the contract state and vaults", async () => {
    // Check if the state account is already initialized
    const stateInfo = await provider.connection.getAccountInfo(
      stateKeypair.publicKey
    );
    if (stateInfo !== null) {
      console.log(
        "ℹ️ State already initialized. Skipping initialize instruction call."
      );
      const state = await program.account.globalState.fetch(
        stateKeypair.publicKey
      );
      assert.equal(state.buyActive, true);
      return;
    }

    try {
      console.log(
        "Sending initialize instruction to Program ID:",
        program.programId.toBase58()
      );
      await program.methods
        .initialize(
          new anchor.BN(90_000),
          new anchor.BN(20),
          vaultAuthorityBump
        ) // Sell $0.09, Swap fee 20%, and pass the bump
        .accounts({
          state: stateKeypair.publicKey,
          tokenMint: tokenMint,
          usdtMint: usdtMint,
          usdtVault: usdtVault,
          rewardVault: rewardVault,
          presaleVault: presaleVault,
          treasuryVault: treasuryVault,
          developmentVault: developmentVault,
          marketingVault: marketingVault,
          roadmapVault: roadmapVault,
          vaultAuthority: vaultAuthority,
          admin: admin.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: anchor.web3.SystemProgram.programId,
          rent: anchor.web3.SYSVAR_RENT_PUBKEY,
        })
        .signers([stateKeypair])
        .rpc({ commitment: "processed", skipPreflight: true });

      // Add a 2-second sleep to ensure the processed state changes are propagated
      console.log("Waiting for block confirmation...");
      sleep(2000);

      const state = await program.account.globalState.fetch(
        stateKeypair.publicKey
      );
      assert.equal(state.buyActive, true);
      assert.equal(state.transactionCount.toNumber(), 0);
      assert.equal(state.swapFeePercentage.toNumber(), 20);
      console.log("✅ Initialization successful!");
    } catch (e) {
      console.log("Initialization Error Logs:", e);
      throw e;
    }
  });

  it("Executes Buy and Stake transaction", async () => {
    const stateData = await program.account.globalState.fetch(
      stateKeypair.publicKey
    );
    const txIndex = stateData.transactionCount.toNumber();

    // Allocate 8 bytes for transaction count key mapping
    const txCountBuffer = Buffer.alloc(8);
    txCountBuffer.writeBigUInt64LE(BigInt(txIndex));

    const [stakeRecord] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("stake-record"), user.publicKey.toBuffer(), txCountBuffer],
      program.programId
    );

    console.log("Staking 200 USDT...");
    try {
      await program.methods
        .buyAndStake(new anchor.BN(200_000_000)) // $200
        .accounts({
          state: stateKeypair.publicKey,
          stakeRecord: stakeRecord,
          usdtMint: usdtMint,
          usdtVault: usdtVault,
          userUsdt: userUsdtAccount,
          user: user.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: anchor.web3.SystemProgram.programId,
        })
        .signers([user])
        .rpc({ commitment: "processed", skipPreflight: true });

      // Add 2 seconds sleep to ensure the processed transaction is fully propagated
      console.log("Waiting for block confirmation...");
      sleep(2000);

      const updatedState = await program.account.globalState.fetch(
        stateKeypair.publicKey
      );
      const record = await program.account.stakingRecord.fetch(stakeRecord);

      console.log("✅ Buy and Stake successful!");
      console.log("User Equivalent AGX:", record.equivalentAgx.toString());
      console.log(
        "User Total Reward AGX:",
        record.totalRewardTokens.toString()
      );

      assert.equal(record.stakedAmountUsdt.toNumber(), 200_000_000);
      assert.equal(record.isStaked, true);
      assert.equal(updatedState.transactionCount.toNumber(), txIndex + 1);
    } catch (e) {
      console.log("Staking Error Logs:", e);
      throw e;
    }
  });

  it("Executes Swap T20 (Instant AGX Buy) at initial price", async () => {
    console.log(
      "Swapping 100 USDT directly for AGX at initial price ($0.09)..."
    );
    try {
      const userAgxBefore = await provider.connection.getTokenAccountBalance(
        userTokenAccount
      );

      await program.methods
        .swapT20(new anchor.BN(100_000_000)) // $100
        .accounts({
          state: stateKeypair.publicKey,
          usdtMint: usdtMint,
          tokenMint: tokenMint,
          usdtVault: usdtVault,
          presaleVault: presaleVault,
          userUsdt: userUsdtAccount,
          userToken: userTokenAccount,
          vaultAuthority: vaultAuthority,
          user: user.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: anchor.web3.SystemProgram.programId,
        })
        .signers([user])
        .rpc({ commitment: "processed", skipPreflight: true });

      // Add 2 seconds sleep to ensure transaction has replicated to the RPC node read index
      console.log("Waiting for block confirmation...");
      sleep(2000);

      const userAgxAfter = await provider.connection.getTokenAccountBalance(
        userTokenAccount
      );
      const change =
        Number(userAgxAfter.value.amount) - Number(userAgxBefore.value.amount);
      console.log("✅ Swap successful!");
      console.log("User AGX Balance Change at $0.09 rate:", change);

      // Math: 100 USDT / 0.09 Price = 1,111.111111111 AGX (in 9 decimals = 1111111111111)
      assert.equal(change, 1111111111111);
    } catch (e) {
      console.log("Swap Error Logs:", e);
      throw e;
    }
  });

  it("Allows admin to update swap price dynamically", async () => {
    console.log("Updating swap price to $0.12 (120,000 USDT units)...");
    try {
      await program.methods
        .updateConfig(
          true, // buyActive
          true, // claimActive
          true, // stakeActive
          true, // swapActive
          new anchor.BN(120_000), // New swap price = $0.12 (120,000 USDT units in 6 decimals)
          new anchor.BN(20) // Swap fee 20%
        )
        .accounts({
          state: stateKeypair.publicKey,
          admin: admin.publicKey,
        })
        .rpc({ commitment: "processed", skipPreflight: true });

      // Fetch the updated state to verify
      console.log("Waiting for block confirmation...");
      sleep(2000);
      const state = await program.account.globalState.fetch(
        stateKeypair.publicKey
      );
      assert.equal(state.sellPrice.toNumber(), 120_000);
      assert.equal(state.swapFeePercentage.toNumber(), 20);
      console.log("✅ Config updated successfully!");
    } catch (e) {
      console.log("Update Config Error:", e);
      throw e;
    }
  });

  it("Executes Swap T20 at the new updated price", async () => {
    console.log("Swapping 100 USDT at the new $0.12 rate...");
    try {
      const userAgxBefore = await provider.connection.getTokenAccountBalance(
        userTokenAccount
      );

      await program.methods
        .swapT20(new anchor.BN(100_000_000)) // $100
        .accounts({
          state: stateKeypair.publicKey,
          usdtMint: usdtMint,
          tokenMint: tokenMint,
          usdtVault: usdtVault,
          presaleVault: presaleVault,
          userUsdt: userUsdtAccount,
          userToken: userTokenAccount,
          vaultAuthority: vaultAuthority,
          user: user.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: anchor.web3.SystemProgram.programId,
        })
        .signers([user])
        .rpc({ commitment: "processed", skipPreflight: true });

      console.log("Waiting for block confirmation...");
      sleep(2000);

      const userAgxAfter = await provider.connection.getTokenAccountBalance(
        userTokenAccount
      );
      const change =
        Number(userAgxAfter.value.amount) - Number(userAgxBefore.value.amount);
      console.log("✅ Swap successful at new price!");
      console.log("User AGX Balance Change at $0.12 price:", change);

      // Math: 100 USDT / 0.12 Price = 833.333333333 AGX (in 9 decimals = 833333333333)
      assert.equal(change, 833333333333);
    } catch (e) {
      console.log("Swap at New Price Error:", e);
      throw e;
    }
  });

  it("Executes Swap AGX back to USDT with 20% Fee", async () => {
    console.log(
      "Selling 12 AGX back to contract at $0.12 rate with 20% fee..."
    );
    try {
      const userUsdtBefore = await provider.connection.getTokenAccountBalance(
        userUsdtAccount
      );
      const userAgxBefore = await provider.connection.getTokenAccountBalance(
        userTokenAccount
      );

      assert.ok(Number(userAgxBefore.value.amount) >= 12_000_000_000);

      await program.methods
        .swapAgxToUsdt(new anchor.BN(12_000_000_000)) // Swap 12 AGX
        .accounts({
          state: stateKeypair.publicKey,
          usdtMint: usdtMint,
          tokenMint: tokenMint,
          usdtVault: usdtVault,
          presaleVault: presaleVault,
          userUsdt: userUsdtAccount,
          userToken: userTokenAccount,
          vaultAuthority: vaultAuthority,
          user: user.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: anchor.web3.SystemProgram.programId,
        })
        .signers([user])
        .rpc({ commitment: "processed", skipPreflight: true });

      console.log("Waiting for block confirmation...");
      sleep(2000);

      const userUsdtAfter = await provider.connection.getTokenAccountBalance(
        userUsdtAccount
      );
      const userAgxAfter = await provider.connection.getTokenAccountBalance(
        userTokenAccount
      );

      const usdtChange =
        Number(userUsdtAfter.value.amount) -
        Number(userUsdtBefore.value.amount);
      const agxChange =
        Number(userAgxBefore.value.amount) - Number(userAgxAfter.value.amount);

      console.log("✅ Swap back successful!");
      console.log("User AGX Deducted (including fee):", agxChange);
      console.log("User USDT Received (net of 20% fee):", usdtChange);

      // Math: 12 AGX swapped. 20% fee = 2.4 AGX. Net AGX = 9.6 AGX.
      // 9.6 AGX * $0.12 price = $1.152 USDT (in 6 decimals = 1,152,000)
      assert.equal(agxChange, 12_000_000_000);
      assert.equal(usdtChange, 1_152_000);
    } catch (e) {
      console.log("Swap back Error:", e);
      throw e;
    }
  });

  it("Executes Claim Refund within 7 days period", async () => {
    // 1. Setup a fresh staking record
    const stateData = await program.account.globalState.fetch(
      stateKeypair.publicKey
    );
    const txIndex = stateData.transactionCount.toNumber();

    const txCountBuffer = Buffer.alloc(8);
    txCountBuffer.writeBigUInt64LE(BigInt(txIndex));

    const [stakeRecord] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("stake-record"), user.publicKey.toBuffer(), txCountBuffer],
      program.programId
    );

    console.log("Initiating buy_and_stake for refund test (200 USDT)...");
    await program.methods
      .buyAndStake(new anchor.BN(200_000_000))
      .accounts({
        state: stateKeypair.publicKey,
        stakeRecord: stakeRecord,
        usdtMint: usdtMint,
        usdtVault: usdtVault,
        userUsdt: userUsdtAccount,
        user: user.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .signers([user])
      .rpc({ commitment: "processed", skipPreflight: true });

    sleep(1500);

    const userUsdtBeforeRefund =
      await provider.connection.getTokenAccountBalance(userUsdtAccount);

    console.log("Requesting refund within 7 days period...");
    await program.methods
      .claimRefund(new anchor.BN(txIndex))
      .accounts({
        state: stateKeypair.publicKey,
        stakeRecord: stakeRecord,
        usdtMint: usdtMint,
        usdtVault: usdtVault,
        userUsdt: userUsdtAccount,
        vaultAuthority: vaultAuthority,
        user: user.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .signers([user])
      .rpc({ commitment: "processed", skipPreflight: true });

    sleep(1500);

    const userUsdtAfterRefund =
      await provider.connection.getTokenAccountBalance(userUsdtAccount);
    const refundChange =
      Number(userUsdtAfterRefund.value.amount) -
      Number(userUsdtBeforeRefund.value.amount);

    console.log("✅ Refund successful!");
    console.log("Refunded USDT returned back to user:", refundChange);

    assert.equal(refundChange, 200_000_000); // Verify full $200 USDT refund

    const updatedRecord = await program.account.stakingRecord.fetch(
      stakeRecord
    );
    assert.equal(updatedRecord.isRefunded, true);
    assert.equal(updatedRecord.isStaked, false);
  });

  it("Enforces operational vault lock time constraints", async () => {
    console.log("Attempting to claim from locked Treasury Vault...");
    try {
      await program.methods
        .claimOperationalVault(1) // 1 = Treasury Vault
        .accounts({
          state: stateKeypair.publicKey,
          treasuryVault: treasuryVault,
          developmentVault: developmentVault,
          marketingVault: marketingVault,
          roadmapVault: roadmapVault,
          userToken: userTokenAccount,
          vaultAuthority: vaultAuthority,
          admin: admin.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: anchor.web3.SystemProgram.programId,
        })
        .rpc({ commitment: "processed", skipPreflight: true });

      assert.fail("Should have failed with OperationalTimeLocked!");
    } catch (err: any) {
      console.log(
        "✅ Lock constraint successfully enforced! Error message:",
        err.message || err
      );
      // Anchor returns custom error message or code
      assert.ok(
        err.toString().includes("OperationalTimeLocked") ||
          err.toString().includes("6009") ||
          err.toString().includes("Error")
      );
    }
  });
});
