/**
 * MAINNET-SAFETY-GATED execution adapter.
 *
 * Required preconditions:
 * - RFQ 1 is on-chain and quoted for flash asset A -> token B.
 * - RFQ 2 is on-chain and quoted for token B -> flash asset A.
 * - Both RFQs are for this executor wallet and their token accounts are its ATAs.
 * - RFQ 1 input equals FLASH_AMOUNT.
 * - RFQ 2 input equals RFQ 1's output.
 * - ALLOW_MAINNET_SUBMISSION=true is set only after a successful simulation.
 */
import BN from "bn.js";
import { getFlashloanContext, getFlashloanIx } from "@jup-ag/lend/flashloan";
import { getAccount, TOKEN_PROGRAM_ID } from "@solana/spl-token";
import {
  ComputeBudgetProgram,
  PublicKey,
  TransactionMessage,
  VersionedTransaction,
} from "@solana/web3.js";
import {
  asBn,
  asPublicKey,
  connection,
  protocolPda,
  publicKeyEnv,
  quotePda,
  requiredEnv,
  rfqPda,
  rfqProgram,
  u64Env,
  vaultAuthorityPda,
} from "./lib.js";

type PairAccount = {
  inputMint: PublicKey;
  outputMint: PublicKey;
  inputVault: PublicKey;
  outputVault: PublicKey;
};

type RfqAccount = {
  taker: PublicKey;
  takerInputAccount: PublicKey;
  takerOutputAccount: PublicKey;
  inputAmount: BN;
  makerPair: PublicKey;
};

type QuoteAccount = {
  outputAmount: BN;
};

function normalisePair(raw: any): PairAccount {
  return {
    inputMint: asPublicKey(raw.inputMint),
    outputMint: asPublicKey(raw.outputMint),
    inputVault: asPublicKey(raw.inputVault),
    outputVault: asPublicKey(raw.outputVault),
  };
}

function normaliseRfq(raw: any): RfqAccount {
  return {
    taker: asPublicKey(raw.taker),
    takerInputAccount: asPublicKey(raw.takerInputAccount),
    takerOutputAccount: asPublicKey(raw.takerOutputAccount),
    inputAmount: asBn(raw.inputAmount),
    makerPair: asPublicKey(raw.makerPair),
  };
}

function normaliseQuote(raw: any): QuoteAccount {
  return { outputAmount: asBn(raw.outputAmount) };
}

async function fillIx(params: {
  program: any;
  config: PublicKey;
  makerPair: PublicKey;
  rfq: PublicKey;
  quote: PublicKey;
  taker: PublicKey;
  takerInput: PublicKey;
  takerOutput: PublicKey;
  pair: PairAccount;
}) {
  const vaultAuthority = vaultAuthorityPda(params.program.programId, params.makerPair);
  return params.program.methods
    .fillQuote()
    .accounts({
      config: params.config,
      makerPair: params.makerPair,
      taker: params.taker,
      rfq: params.rfq,
      quote: params.quote,
      takerInputAccount: params.takerInput,
      takerOutputAccount: params.takerOutput,
      makerInputVault: params.pair.inputVault,
      makerOutputVault: params.pair.outputVault,
      vaultAuthority,
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .instruction();
}

async function main() {
  const program = rfqProgram();
  const rpc = connection();
  const executor = program.provider.publicKey;
  const assetA = publicKeyEnv("FLASH_ASSET_MINT");
  const flashAmount = u64Env("FLASH_AMOUNT");
  const minProfit = u64Env("MIN_PROFIT_A");

  const pairAB = publicKeyEnv("MAKER_PAIR_AB");
  const pairBA = publicKeyEnv("MAKER_PAIR_BA");
  const rfqOneId = u64Env("RFQ_ONE_ID");
  const rfqTwoId = u64Env("RFQ_TWO_ID");
  const tokenAccountA = publicKeyEnv("TAKER_A_TOKEN_ACCOUNT");
  const tokenAccountB = publicKeyEnv("TAKER_B_TOKEN_ACCOUNT");
  const config = protocolPda(program.programId);

  const rfqOne = rfqPda(program.programId, executor, rfqOneId);
  const rfqTwo = rfqPda(program.programId, executor, rfqTwoId);
  const quoteOne = quotePda(program.programId, rfqOne);
  const quoteTwo = quotePda(program.programId, rfqTwo);

  const [pairOneRaw, pairTwoRaw, rfqOneRaw, rfqTwoRaw, quoteOneRaw, quoteTwoRaw] = await Promise.all([
    program.account.makerPairConfig.fetch(pairAB),
    program.account.makerPairConfig.fetch(pairBA),
    program.account.rfqRequest.fetch(rfqOne),
    program.account.rfqRequest.fetch(rfqTwo),
    program.account.quote.fetch(quoteOne),
    program.account.quote.fetch(quoteTwo),
  ]);

  const pairOne = normalisePair(pairOneRaw);
  const pairTwo = normalisePair(pairTwoRaw);
  const requestOne = normaliseRfq(rfqOneRaw);
  const requestTwo = normaliseRfq(rfqTwoRaw);
  const selectedQuoteOne = normaliseQuote(quoteOneRaw);
  const selectedQuoteTwo = normaliseQuote(quoteTwoRaw);

  // Fail closed before asking Jupiter Lend for instructions.
  if (!pairOne.inputMint.equals(assetA)) throw new Error("Pair AB input must be FLASH_ASSET_MINT");
  if (!pairOne.outputMint.equals(pairTwo.inputMint)) throw new Error("Pair AB output must equal pair BA input");
  if (!pairTwo.outputMint.equals(assetA)) throw new Error("Pair BA output must return to FLASH_ASSET_MINT");
  if (!requestOne.taker.equals(executor) || !requestTwo.taker.equals(executor)) throw new Error("RFQs are not bound to executor");
  if (!requestOne.makerPair.equals(pairAB) || !requestTwo.makerPair.equals(pairBA)) throw new Error("RFQ pair mismatch");
  if (!requestOne.takerInputAccount.equals(tokenAccountA) || !requestOne.takerOutputAccount.equals(tokenAccountB)) throw new Error("RFQ 1 token account mismatch");
  if (!requestTwo.takerInputAccount.equals(tokenAccountB) || !requestTwo.takerOutputAccount.equals(tokenAccountA)) throw new Error("RFQ 2 token account mismatch");
  if (!requestOne.inputAmount.eq(flashAmount)) throw new Error("RFQ 1 input must exactly equal FLASH_AMOUNT");
  if (!requestTwo.inputAmount.eq(selectedQuoteOne.outputAmount)) throw new Error("RFQ 2 input must exactly equal RFQ 1 quoted output");

  const market = (process.env.JUPITER_LEND_MARKET ?? "main") as "main" | "ethena" | "galaxy";
  const flashContext = await getFlashloanContext({
    connection: rpc,
    signer: executor,
    asset: assetA,
    market,
  });
  if (!flashContext.signerBorrowTokenAccount.equals(tokenAccountA)) {
    throw new Error(
      `Jupiter Lend will borrow into ${flashContext.signerBorrowTokenAccount.toBase58()}, not TAKER_A_TOKEN_ACCOUNT. Use that account when creating both RFQs.`,
    );
  }

  const { borrowIx, paybackIx, setupIxs, cleanupIxs, feeAmount } = await getFlashloanIx({
    connection: rpc,
    signer: executor,
    asset: assetA,
    amount: flashAmount,
    market,
    includeATASetup: false,
    includeWrapSol: false,
  });
  if (setupIxs.length || cleanupIxs.length) throw new Error("This lab requires pre-created token accounts; disable Jupiter setup/cleanup instructions.");

  const initialA = (await getAccount(rpc, tokenAccountA)).amount;
  const requiredBeforePayback = new BN(initialA.toString())
    .add(flashAmount)
    .add(feeAmount)
    .add(minProfit);

  // Cheap preflight economics check. The program assertion below is the authoritative guard.
  const expectedRoundTripOut = selectedQuoteTwo.outputAmount;
  const requiredRoundTripOut = flashAmount.add(feeAmount).add(minProfit);
  if (expectedRoundTripOut.lt(requiredRoundTripOut)) {
    throw new Error(
      `Not profitable from quoted amounts: RFQ2 out ${expectedRoundTripOut} < principal + fee + min profit ${requiredRoundTripOut}`,
    );
  }

  const [fillOne, fillTwo, assertProfit] = await Promise.all([
    fillIx({
      program,
      config,
      makerPair: pairAB,
      rfq: rfqOne,
      quote: quoteOne,
      taker: executor,
      takerInput: tokenAccountA,
      takerOutput: tokenAccountB,
      pair: pairOne,
    }),
    fillIx({
      program,
      config,
      makerPair: pairBA,
      rfq: rfqTwo,
      quote: quoteTwo,
      taker: executor,
      takerInput: tokenAccountB,
      takerOutput: tokenAccountA,
      pair: pairTwo,
    }),
    program.methods
      .assertMinTokenBalance(requiredBeforePayback)
      .accounts({ taker: executor, tokenAccount: tokenAccountA })
      .instruction(),
  ]);

  const blockhash = await rpc.getLatestBlockhash("confirmed");
  const instructions = [
    ComputeBudgetProgram.setComputeUnitLimit({ units: Number(process.env.COMPUTE_UNIT_LIMIT ?? "1000000") }),
    ...(process.env.COMPUTE_UNIT_PRICE_MICROLAMPORTS
      ? [ComputeBudgetProgram.setComputeUnitPrice({ microLamports: Number(process.env.COMPUTE_UNIT_PRICE_MICROLAMPORTS) })]
      : []),
    borrowIx,
    fillOne,
    fillTwo,
    assertProfit,
    paybackIx,
  ];
  const message = new TransactionMessage({
    payerKey: executor,
    recentBlockhash: blockhash.blockhash,
    instructions,
  }).compileToV0Message();
  const transaction = new VersionedTransaction(message);
  transaction.sign([(program.provider.wallet as any).payer]);

  const simulation = await rpc.simulateTransaction(transaction, { sigVerify: false, commitment: "confirmed" });
  if (simulation.value.err) {
    console.error("Simulation logs:\n", simulation.value.logs?.join("\n"));
    throw new Error(`Simulation failed: ${JSON.stringify(simulation.value.err)}`);
  }
  console.log("Simulation succeeded.", {
    expectedRoundTripOut: expectedRoundTripOut.toString(),
    flashFee: feeAmount.toString(),
    minProfit: minProfit.toString(),
    requiredBeforePayback: requiredBeforePayback.toString(),
    unitsConsumed: simulation.value.unitsConsumed,
  });

  if (process.env.ALLOW_MAINNET_SUBMISSION !== "true") {
    console.log("Dry run only. Set ALLOW_MAINNET_SUBMISSION=true only after independently reviewing the simulation and account configuration.");
    return;
  }

  const signature = await rpc.sendTransaction(transaction, { skipPreflight: false, maxRetries: 2, preflightCommitment: "confirmed" });
  await rpc.confirmTransaction({ signature, ...blockhash }, "confirmed");
  console.log("Submitted atomically:", signature);
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
