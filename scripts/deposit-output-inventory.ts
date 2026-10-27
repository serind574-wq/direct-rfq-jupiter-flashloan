/** Runs as a maker owner and funds the output vault used to honour on-chain quotes. */
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { asPublicKey, publicKeyEnv, rfqProgram, u64Env } from "./lib.js";

async function main() {
  const program = rfqProgram();
  const makerOwner = program.provider.publicKey;
  const makerPair = publicKeyEnv("MAKER_PAIR");
  const makerOutputSource = publicKeyEnv("MAKER_OUTPUT_SOURCE");
  const amount = u64Env("DEPOSIT_AMOUNT");
  const pair = await program.account.makerPairConfig.fetch(makerPair);

  const signature = await program.methods
    .depositOutputInventory(amount)
    .accounts({
      makerPair,
      makerOwner,
      makerOutputSource,
      makerOutputVault: asPublicKey(pair.outputVault),
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .rpc();

  console.log(JSON.stringify({ signature, makerPair: makerPair.toBase58(), amount: amount.toString() }, null, 2));
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
