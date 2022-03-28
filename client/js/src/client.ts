import { Account } from "./account";
import { Block } from "./block";
import { Pubkey } from "./pubkey";
import { Transaction, TransactionHash, TransactionResult } from "./transaction";

import fetch from 'cross-fetch';

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

  async getNextAccountNonce(address: Pubkey): Promise<number> {
    const account = await this.getAccount(address, Commitment.Confirmed);
    return account !== null ? account.nonce + 1 : 1;
  }

  async sendTransactions(transaction: Transaction[]): Promise<Record<TransactionHash, string>[]> {
    const result = await fetch(`${this.host}/transactions`, {
      method: 'POST',
      headers: { "content-type": "application/json" },
      body: JSON.stringify(transaction)
    });
    if (result.status == 202) {
      return await result.json() as Record<TransactionHash, string>[];
    } else {
      throw Error(`server error: ${await result.text()}`);
    }
  }

  async confirmTransaction(
    txhash: string,
    commitment: Commitment = Commitment.Confirmed
  ): Promise<TransactionResult> {
    const waitms = 500;
    while (true) {
      let txresult = await this.getTransaction(txhash);
      if (txresult !== null) {
        if (commitment === Commitment.Finalized &&
          txresult.commitment.toLocaleLowerCase() !== Commitment.Finalized) {
          // the transaction is confirmed, but not finalized
          // yet, wait for it to become finalized.
          await new Promise(f => setTimeout(f, waitms));
        } else {
          return txresult;
        }
      } else {
        await new Promise(f => setTimeout(f, waitms));
      }
    }
  }

  async sendAndConfirmTransactions(
    transaction: Transaction[],
    commitment: Commitment = Commitment.Confirmed
  ): Promise<(string | TransactionResult)[]> {
    let txs = await this.sendTransactions(transaction);
    let output = [];
    for (let tx of txs) {
      for (const [txhash, result] of Object.entries(tx)) {
        if (result === "ok") {
          output.push(await this.confirmTransaction(txhash, commitment));
        } else {
          output.push(result)
        }
      }
    }
    return output;
  }

  async getTransaction(hash: TransactionHash): Promise<TransactionResult | null> {
    const result = await fetch(`${this.host}/transaction/${hash}`);
    if (result.status == 200) {
      let obj = await result.json();
      return obj as TransactionResult;
    } else if (result.status == 404) {
      return null;
    } else {
      throw Error(`invalid return code ${result.status} from server`);
    }
  }

  async getLatestBlock(commitment: Commitment): Promise<Block> {
    const result = await fetch(`${this.host}/info`);
    const obj = await result.json() as any;

    var latestBlock;
    if (commitment === Commitment.Confirmed) {
      latestBlock = obj["confirmed"]["height"] as number;
    } else {
      latestBlock = obj["finalized"]["height"] as number;
    }

    const blockResult = await fetch(`${this.host}/block/${latestBlock}`);
    return await blockResult.json() as any;
  }

  async getAccount(address: Pubkey, commitment: Commitment = Commitment.Confirmed): Promise<Account | null> {
    const commutmentQuery = `?commitment=${commitment}`;
    const result = await fetch(`${this.host}/account/${address.toString()}${commutmentQuery}`);
    if (result.status == 200) {
      let obj = await result.json() as any;
      return obj['account'] as Account;
    } else if (result.status == 404) {
      return null
    } else {
      throw Error(`invalid return code ${result.status} from server: ${await result.text()}`);
    }
  }
}