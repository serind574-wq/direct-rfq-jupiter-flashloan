/**
 * Creates an on-chain RFQ. Run it once for token A -> B and then again for B -> A.
 * The maker posts its quote in a separate transaction before the final flash-loan tx.
 */
import BN from "bn.js";
import { SystemProgram } from "@solana/web3.js";
import { connection, protocolPda, publicKeyEnv, requiredEnv, rfqPda, rfqProgram, u64Env } from "./lib.js";

async function main() {
  const program = rfqProgram();
  const rpc = connection();
  const taker = program.provider.publicKey;
  const makerPair = publicKeyEnv("MAKER_PAIR");
  const takerInput = publicKeyEnv("TAKER_INPUT_TOKEN_ACCOUNT");
  const takerOutput = publicKeyEnv("TAKER_OUTPUT_TOKEN_ACCOUNT");
  const requestId = u64Env("REQUEST_ID");
  const inputAmount = u64Env("INPUT_AMOUNT");
  const minOutputAmount = u64Env("MIN_OUTPUT_AMOUNT");
  const ttlSlots = new BN(process.env.RFQ_TTL_SLOTS ?? "30");

  const currentSlot = await rpc.getSlot("confirmed");
  const expirySlot = new BN(currentSlot).add(ttlSlots);
  const config = protocolPda(program.programId);
  const rfq = rfqPda(program.programId, taker, requestId);

  const signature = await program.methods
    .createRfq({
      requestId,
      inputAmount,
      minOutputAmount,
      expirySlot,
    })
    .accounts({
      config,
      makerPair,
      taker,
      takerInputAccount: takerInput,
      takerOutputAccount: takerOutput,
      rfq,
      systemProgram: SystemProgram.programId,
    })
    .rpc();

  console.log(JSON.stringify({ signature, rfq: rfq.toBase58(), expirySlot: expirySlot.toString() }, null, 2));
  console.log("Next: the fixed maker must run `npm run post-quote` using its maker-owner keypair.");
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
