import bs58 from "bs58";
import SHA3 from "sha3";
import { Keypair, Pubkey } from "./pubkey";

export type TransactionHash = String;

export interface AccountRef {
  signer: boolean;
  writable: boolean;
  address: string;
};

export interface Transaction {
  nonce: number;
  contract: string;
  accounts: AccountRef[];
  payer: string,
  params: string,
  signatures: string[]
}

export async function createTransaction(
  contract: Pubkey,
  nonce: number,
  payer: Keypair,
  accounts: [AccountRef],
  signers: [Keypair],
  params: Uint8Array): Promise<Transaction> {

  const hasher = new SHA3(256);
  hasher.update(Buffer.from(contract.bytes));

  // nonce as le bytes
  const buffer = new ArrayBuffer(8);
  new DataView(buffer).setBigUint64(0, BigInt(nonce), true);
  hasher.update(Buffer.from(buffer));
  hasher.update(Buffer.from(payer.publicKey.bytes));

  for (const acc of accounts) {
    hasher.update(Buffer.from(bs58.decode(acc.address)));
    hasher.update(acc.writable ? Buffer.from([0x1]) : Buffer.from([0x0]));
    hasher.update(acc.signer ? Buffer.from([0x1]) : Buffer.from([0x0]));
  }
  hasher.update(Buffer.from(params));
  const digest = hasher.digest();

  return {
    nonce: nonce,
    contract: contract.toString(),
    accounts: accounts.map((acc) => ({
      address: acc.address,
      signer: acc.signer,
      writable: acc.writable,
    })),
    payer: payer.publicKey.toString(),
    params: bs58.encode(params),
    signatures: [
      bs58.encode(await payer.sign(digest)),
    ].concat(await Promise.all(signers.map(async (s) => bs58.encode(await s.sign(digest)))))
  };
}

export interface TransactionResult { }