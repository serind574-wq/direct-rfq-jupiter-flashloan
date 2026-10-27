/**
 * Registers a direct on-chain RFQ pair and creates PDA-owned input/output vault ATAs.
 * For a local study, protocol authority and maker may be the same key. In a real setup,
 * authority is normally a multisig and maker_owner is a separate signer.
 */
import { getAssociatedTokenAddress, ASSOCIATED_TOKEN_PROGRAM_ID, TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { SystemProgram } from "@solana/web3.js";
import {
  loadKeypair,
  makerPairPda,
  protocolPda,
  publicKeyEnv,
  rfqProgram,
  u64Env,
  vaultAuthorityPda,
} from "./lib.js";

async function main() {
  const program = rfqProgram();
  const authority = program.provider.publicKey;
  const maker = process.env.MAKER_OWNER_KEYPAIR_PATH
    ? loadKeypair(process.env.MAKER_OWNER_KEYPAIR_PATH)
    : (program.provider.wallet as any).payer;

  const inputMint = publicKeyEnv("INPUT_MINT");
  const outputMint = publicKeyEnv("OUTPUT_MINT");
  const maxInputAmount = u64Env("MAX_INPUT_AMOUNT");
  const makerPair = makerPairPda(program.programId, maker.publicKey, inputMint, outputMint);
  const vaultAuthority = vaultAuthorityPda(program.programId, makerPair);
  const makerInputVault = await getAssociatedTokenAddress(inputMint, vaultAuthority, true);
  const makerOutputVault = await getAssociatedTokenAddress(outputMint, vaultAuthority, true);

  const builder = program.methods.registerMakerPair(maxInputAmount).accounts({
    config: protocolPda(program.programId),
    authority,
    makerOwner: maker.publicKey,
    inputMint,
    outputMint,
    makerPair,
    vaultAuthority,
    makerInputVault,
    makerOutputVault,
    tokenProgram: TOKEN_PROGRAM_ID,
    associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
    systemProgram: SystemProgram.programId,
  });

  const signature = maker.publicKey.equals(authority) ? await builder.rpc() : await builder.signers([maker]).rpc();
  console.log(
    JSON.stringify(
      {
        signature,
        makerPair: makerPair.toBase58(),
        vaultAuthority: vaultAuthority.toBase58(),
        makerInputVault: makerInputVault.toBase58(),
        makerOutputVault: makerOutputVault.toBase58(),
      },
      null,
      2,
    ),
  );
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
