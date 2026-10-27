import "dotenv/config";
import fs from "node:fs";
import path from "node:path";
import BN from "bn.js";
import { AnchorProvider, Idl, Program, Wallet } from "@coral-xyz/anchor";
import { Connection, Keypair, PublicKey } from "@solana/web3.js";

export const PROGRAM_SEED = Buffer.from("protocol");

export function requiredEnv(name: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`Missing required environment variable: ${name}`);
  return value;
}

export function publicKeyEnv(name: string): PublicKey {
  return new PublicKey(requiredEnv(name));
}

export function u64Env(name: string): BN {
  const value = requiredEnv(name);
  if (!/^\d+$/.test(value)) throw new Error(`${name} must be an unsigned integer in base units`);
  return new BN(value, 10);
}

export function loadKeypair(keypairPath = requiredEnv("KEYPAIR_PATH")): Keypair {
  const fullPath = path.resolve(keypairPath);
  return Keypair.fromSecretKey(new Uint8Array(JSON.parse(fs.readFileSync(fullPath, "utf8"))));
}

export function connection(): Connection {
  return new Connection(process.env.RPC_URL ?? "https://api.mainnet-beta.solana.com", "confirmed");
}

export function provider(walletKeypair = loadKeypair()): AnchorProvider {
  const rpc = connection();
  return new AnchorProvider(rpc, new Wallet(walletKeypair), { commitment: "confirmed" });
}

/** Requires `anchor build` first because it loads the generated IDL. */
// The IDL is generated at build time, so scripts intentionally use `any` here.
// The on-chain account and instruction names are still documented and tested by Anchor.
export function rfqProgram(providerOverride?: AnchorProvider): any {
  const activeProvider = providerOverride ?? provider();
  const idlPath = path.resolve("target/idl/direct_rfq_arb.json");
  if (!fs.existsSync(idlPath)) {
    throw new Error("IDL is missing. Run `anchor build` before using client scripts.");
  }

  const idl = JSON.parse(fs.readFileSync(idlPath, "utf8")) as Idl & { address?: string };
  idl.address = process.env.RFQ_PROGRAM_ID ?? idl.address;
  if (!idl.address) throw new Error("IDL has no address; set RFQ_PROGRAM_ID.");
  return new Program(idl, activeProvider) as any;
}

export function protocolPda(programId: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync([PROGRAM_SEED], programId)[0];
}

export function rfqPda(programId: PublicKey, taker: PublicKey, requestId: BN): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("rfq"), taker.toBuffer(), requestId.toArrayLike(Buffer, "le", 8)],
    programId,
  )[0];
}

export function quotePda(programId: PublicKey, rfq: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync([Buffer.from("quote"), rfq.toBuffer()], programId)[0];
}

export function makerPairPda(
  programId: PublicKey,
  makerOwner: PublicKey,
  inputMint: PublicKey,
  outputMint: PublicKey,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("maker-pair"), makerOwner.toBuffer(), inputMint.toBuffer(), outputMint.toBuffer()],
    programId,
  )[0];
}

export function vaultAuthorityPda(programId: PublicKey, makerPair: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("vault-authority"), makerPair.toBuffer()],
    programId,
  )[0];
}

export function asPublicKey(value: unknown): PublicKey {
  if (value instanceof PublicKey) return value;
  return new PublicKey(value as string);
}

export function asBn(value: unknown): BN {
  if (BN.isBN(value)) return value;
  if (typeof value === "bigint") return new BN(value.toString());
  return new BN(String(value));
}
