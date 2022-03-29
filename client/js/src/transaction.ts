import bs58 from "bs58";
import SHA3 from "sha3";
import { Client, Commitment } from "./client";
import { Keypair, Pubkey } from "./pubkey";

export type TransactionHash = string;

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

export interface TransactionCreationParams {
  contract: Pubkey;
  payer: Keypair;
  accounts: AccountRef[];
  signers: Keypair[];
  params: Uint8Array;
}

export async function createManyTransactions(
  client: Client,
  inputs: TransactionCreationParams[]
): Promise<Transaction[]> {
  let transactions = [];
  let nonces: Record<string, number> = {};
  for (var input of inputs) {
    if (nonces[input.payer.publicKey.toString()] === undefined) {
      nonces[input.payer.publicKey.toString()] =
        await client.getNextAccountNonce(input.payer.publicKey);
    } else {
      nonces[input.payer.publicKey.toString()] += 1;
    }
    let txnonce = nonces[input.payer.publicKey.toString()];
    transactions.push(await createTransaction(txnonce, input));
  }
  return transactions;
}

export async function createTransaction(
  clientOrNonce: Client | number,
  input: TransactionCreationParams
): Promise<Transaction> {
  let nonce = 0;
  if (clientOrNonce instanceof Client) {
    nonce = await clientOrNonce.getNextAccountNonce(input.payer.publicKey);
  } else {
    nonce = clientOrNonce;
  }

  const hasher = new SHA3(256);
  hasher.update(Buffer.from(input.contract.bytes));

  // nonce as le bytes
  const buffer = new ArrayBuffer(8);
  new DataView(buffer).setBigUint64(0, BigInt(nonce), true);
  hasher.update(Buffer.from(buffer));
  hasher.update(Buffer.from(input.payer.publicKey.bytes));

  for (const acc of input.accounts) {
    hasher.update(Buffer.from(bs58.decode(acc.address)));
    hasher.update(acc.writable ? Buffer.from([0x1]) : Buffer.from([0x0]));
    hasher.update(acc.signer ? Buffer.from([0x1]) : Buffer.from([0x0]));
  }
  hasher.update(Buffer.from(input.params));
  const digest = hasher.digest();

  return {
    nonce: nonce,
    contract: input.contract.toString(),
    accounts: input.accounts.map((acc) => ({
      address: acc.address,
      signer: acc.signer,
      writable: acc.writable,
    })),
    payer: input.payer.publicKey.toString(),
    params: bs58.encode(input.params),
    signatures: [
      bs58.encode(await input.payer.sign(digest)),
    ].concat(await Promise.all(input.signers.map(async (s) => bs58.encode(await s.sign(digest)))))
  };
}

export interface TransactionResult {
  block: number;
  commitment: Commitment;
  hash: string;
  output: any;
  transaction: Transaction;
}