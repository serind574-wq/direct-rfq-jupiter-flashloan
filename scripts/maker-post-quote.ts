/**
 * Runs as the fixed maker. It posts an executable on-chain quote and reserves
 * output inventory in the maker-pair vault.
 */
import BN from "bn.js";
import { SystemProgram } from "@solana/web3.js";
import {
  asPublicKey,
  connection,
  protocolPda,
  publicKeyEnv,
  quotePda,
  requiredEnv,
  rfqPda,
  rfqProgram,
  u64Env,
} from "./lib.js";

async function main() {
  const program = rfqProgram();
  const rpc = connection();
  const makerOwner = program.provider.publicKey;
  const makerPair = publicKeyEnv("MAKER_PAIR");
  const taker = publicKeyEnv("TAKER_PUBKEY");
  const requestId = u64Env("REQUEST_ID");
  const outputAmount = u64Env("QUOTE_OUTPUT_AMOUNT");
  const ttlSlots = new BN(process.env.QUOTE_TTL_SLOTS ?? "12");

  const rfq = rfqPda(program.programId, taker, requestId);
  const quote = quotePda(program.programId, rfq);
  const pair = await program.account.makerPairConfig.fetch(makerPair);
  const currentSlot = await rpc.getSlot("confirmed");
  const expirySlot = new BN(currentSlot).add(ttlSlots);

  const signature = await program.methods
    .postQuote({ outputAmount, expirySlot })
    .accounts({
      config: protocolPda(program.programId),
      makerPair,
      makerOwner,
      rfq,
      quote,
      makerOutputVault: asPublicKey(pair.outputVault),
      systemProgram: SystemProgram.programId,
    })
    .rpc();

  console.log(JSON.stringify({ signature, rfq: rfq.toBase58(), quote: quote.toBase58(), expirySlot: expirySlot.toString() }, null, 2));
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
