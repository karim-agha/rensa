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

  async mint(client: Client, to: Pubkey, authority: Keypair, amount: number): Promise<Transaction> {
    let nonce = await client.getNextAccountNonce(authority.publicKey);

    let accounts = [
      // mint address
      {
        address: this.mintAddress.toString(),
        writable: true,
        signer: false
      },
      // mint authority as signer
      {
        address: authority.publicKey.toString(),
        writable: false,
        signer: true
      },
      // recipient wallet owner address
      {
        address: to.toString(),
        writable: false,
        signer: false
      },
      // recipient coin account
      {
        address: CURRENCY_CONTRACT_ADDR.derive([this.mintAddress, to]).toString(),
        writable: true,
        signer: false
      }
    ];

    // params in BORSH format
    const writer = new BinaryWriter();
    writer.writeU8(1);
    writer.writeU64(amount);

    return createTransaction(
      CURRENCY_CONTRACT_ADDR,
      nonce,
      authority,
      accounts,
      [authority],
      writer.toArray()
    );
  }

  async transfer(client: Client, from: Keypair, to: Pubkey, amount: number): Promise<Transaction> {
    let nonce = await client.getNextAccountNonce(from.publicKey);

    let accounts = [
      // mint address
      {
        address: this.mintAddress.toString(),
        writable: false,
        signer: false
      },
      // sender wallet owner
      {
        address: from.publicKey.toString(),
        writable: false,
        signer: true
      },
      // sender coin address
      {
        address: CURRENCY_CONTRACT_ADDR.derive([this.mintAddress, from.publicKey]).toString(),
        writable: true,
        signer: false
      },
      // recipient wallet owner
      {
        address: to.toString(),
        writable: false,
        signer: false
      },
      // recipient coin address
      {
        address: CURRENCY_CONTRACT_ADDR.derive([this.mintAddress, to]).toString(),
        writable: true,
        signer: false
      }
    ];

    // params in BORSH format
    const writer = new BinaryWriter();
    writer.writeU8(2);
    writer.writeU64(amount);

    return createTransaction(
      CURRENCY_CONTRACT_ADDR,
      nonce,
      from,
      accounts,
      [from],
      writer.toArray()
    );

  }

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

    // instruction index
    writer.writeU8(0);

    // seed
    writer.writeFixedArray(seed);

    // authority pubkey
    writer.writeFixedArray(authority.publicKey.bytes);

    // decimals
    writer.writeU8(decimals);

    // optional name
    if (name === null) {
      writer.writeU8(0);
    } else {
      writer.writeU8(1);
      writer.writeString(name);
    }

    // optional symbol
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
}

