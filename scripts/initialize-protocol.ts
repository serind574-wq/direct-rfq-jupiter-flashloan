import { SystemProgram } from "@solana/web3.js";
import { protocolPda, rfqProgram } from "./lib.js";

async function main() {
  const program = rfqProgram();
  const authority = program.provider.publicKey;
  const config = protocolPda(program.programId);
  const maxRequestTtlSlots = Number(process.env.MAX_REQUEST_TTL_SLOTS ?? "100");

  const signature = await program.methods
    .initializeProtocol(maxRequestTtlSlots)
    .accounts({ authority, config, systemProgram: SystemProgram.programId })
    .rpc();

  console.log(JSON.stringify({ signature, config: config.toBase58(), maxRequestTtlSlots }, null, 2));
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
