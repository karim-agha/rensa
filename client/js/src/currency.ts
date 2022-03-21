import { BinaryWriter } from "borsh";
import { Client } from "./client";
import { Keypair, Pubkey } from "./pubkey";
import { createTransaction, Transaction, TransactionHash } from "./transaction";

const CURRENCY_CONTRACT_ADDR = new Pubkey("Currency1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx");

export class Currency {
  mintAddress: Pubkey;

  constructor(mint: Pubkey) {
    this.mintAddress = mint;
  }

  // async mint(to: Pubkey, authority: Keypair, amount: number): Promise<Transaction> {

  // }

  // async transfer(from: Keypair, to: Pubkey, amount: number): Promise<Transaction> {

  // }

  // async burn(from: Keypair, amount: number): Promise<Transaction> {

  // }

  /**
   * Creates a new transaction that initiates a new coin type.
   * 
   * @param seed bytes that are used to derive the mint address
   * @param authority public key of the account allowed to mint new coins and the payer of the tx fees
   * @param decimals number of decimals this coin has
   * @param name optional human readable name of the coin
   * @param symbol optional human readable symbol of the coin
   */
  static async create(
    client: Client,
    seed: Uint8Array,
    authority: Keypair,
    decimals: number,
    name: string | null,
    symbol: string | null): Promise<Transaction> {
    let mintAddress = CURRENCY_CONTRACT_ADDR.derive([seed]);

    let nonce = await client.getNextAccountNonce(authority.publicKey);

    // params in BORSH format
    const writer = new BinaryWriter();
    writer.writeU8(0);
    writer.writeFixedArray(seed);
    writer.writeFixedArray(authority.publicKey.bytes);
    writer.writeU8(decimals);
    if (name === null) {
      writer.writeU8(0);
    } else {
      writer.writeU8(1);
      writer.writeString(name);
    }

    if (symbol === null) {
      writer.writeU8(0);
    } else {
      writer.writeU8(1);
      writer.writeString(symbol);
    }

    return createTransaction(
      CURRENCY_CONTRACT_ADDR,
      nonce,
      authority,
      [{
        address: mintAddress.toString(),
        signer: false,
        writable: true
      }],
      [],
      writer.toArray()
    );
  }

  // static async fromCreateTransaction(client: Client, tx: TransactionHash): Promise<Currency> {

  // }
}

