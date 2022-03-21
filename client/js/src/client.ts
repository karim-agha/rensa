import { Account } from "./account";
import { Block } from "./block";
import { Pubkey } from "./pubkey";
import { Transaction, TransactionHash, TransactionResult } from "./transaction";

export enum Commitment {
  Confirmed = "confirmed",
  Finalized = "finalized"
}

/**
 * Represents an RPC client that sends transactions to a known RPC
 * endpoint and queries it for various information.
 */
export class Client {
  private host: String;

  constructor(host: String) {
    this.host = host;
  }

  async sendTransaction(transaction: Transaction): Promise<TransactionHash> {
    return "";
  }

  async getTransaction(hash: TransactionHash): Promise<TransactionResult> {
    return {};
  }

  async getLatestBlock(commitment: Commitment): Promise<Block> {
    return {};
  }

  async getAccount(address: Pubkey, commitment: Commitment | null): Promise<Account> {
    return {};
  }
}