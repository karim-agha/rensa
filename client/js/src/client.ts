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

  async sendTransaction(transaction: Transaction): Promise<TransactionHash> {
    const result = await fetch(`${this.host}/transaction`, {
      method: 'POST',
      headers: { "content-type": "application/json" },
      body: JSON.stringify(transaction)
    });
    if (result.status == 201) {
      const res = await result.json() as any;
      return res["transaction"] as TransactionHash;
    } else if (result.status == 400) {
      const res = await result.json() as any;
      throw Error(res["error"]);
    } else {
      throw Error(`server error: ${await result.text()}`);
    }
  }

  async sendAndConfirmTransaction(transaction: Transaction): Promise<TransactionResult> {
    let txhash = await this.sendTransaction(transaction);
    while (true) {
      let txresult = await this.getTransaction(txhash);
      if (txresult !== null) {
        return txresult;
      } else {
        await new Promise(f => setTimeout(f, 500));
      }
    }
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

  async getAccount(address: Pubkey, commitment: Commitment | null): Promise<Account | null> {
    const commutmentQuery = commitment === null ? "" : `?commitment=${commitment}`;
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