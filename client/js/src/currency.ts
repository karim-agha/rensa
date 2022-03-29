import BN from "bn.js";
import { BinaryReader, BinaryWriter } from "borsh";
import { decode } from "bs58";
import { Client, Commitment } from "./client";
import { Keypair, Pubkey } from "./pubkey";
import { TransactionCreationParams } from "./transaction";

const CURRENCY_CONTRACT_ADDR = new Pubkey("Currency1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx");

export class Currency {
  mintAddress: Pubkey;

  constructor(mint: Pubkey) {
    this.mintAddress = mint;
  }

  async balance(client: Client, wallet: Pubkey, commitment: Commitment = Commitment.Confirmed): Promise<BN> {
    let coinAddress = CURRENCY_CONTRACT_ADDR.derive([this.mintAddress, wallet]);
    let coinAccount = await client.getAccount(coinAddress, commitment);

    if (coinAccount === null) {
      return new BN(0);
    } else {
      if (coinAccount.owner !== CURRENCY_CONTRACT_ADDR.toString()) {
        throw Error("unexpected coin account owner");
      }

      const coinData = decode(coinAccount.data);
      const reader = new BinaryReader(Buffer.from(coinData));
      const mintAddress = new Pubkey(reader.readFixedArray(32));

      if (!mintAddress.equals(this.mintAddress)) {
        throw Error("unexpected mint address for this coin account: " + mintAddress.toString());
      }

      const ownerAddress = new Pubkey(reader.readFixedArray(32));
      if (!ownerAddress.equals(wallet)) {
        throw Error("unexpected wallet owner for this coin account: " + ownerAddress.toString());
      }

      const balance = reader.readU64();
      return balance;
    }
  }

  mint(to: Pubkey, authority: Keypair, payer: Keypair, amount: number): TransactionCreationParams {
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

    return {
      contract: CURRENCY_CONTRACT_ADDR,
      payer: payer,
      accounts: accounts,
      signers: [authority],
      params: writer.toArray()
    };
  }

  transfer(from: Keypair, to: Pubkey, amount: number): TransactionCreationParams {
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

    return {
      contract: CURRENCY_CONTRACT_ADDR,
      payer: from,
      accounts: accounts,
      signers: [from],
      params: writer.toArray()
    };

  }

  burn(wallet: Keypair, amount: number): TransactionCreationParams {

    let accounts = [
      // mint address
      {
        address: this.mintAddress.toString(),
        writable: true,
        signer: false
      },
      // wallet owner
      {
        address: wallet.publicKey.toString(),
        writable: false,
        signer: true
      },
      // wallet coin address
      {
        address: CURRENCY_CONTRACT_ADDR.derive([this.mintAddress, wallet.publicKey]).toString(),
        writable: true,
        signer: false
      }
    ];

    // params in BORSH format
    const writer = new BinaryWriter();
    writer.writeU8(3);
    writer.writeU64(amount);

    return {
      contract: CURRENCY_CONTRACT_ADDR,
      payer: wallet,
      accounts: accounts,
      signers: [wallet],
      params: writer.toArray()
    };
  }

  /**
   * Creates a new transaction that initiates a new coin type.
   * 
   * @param seed bytes that are used to derive the mint address
   * @param authority public key of the account allowed to mint new coins and the payer of the tx fees
   * @param decimals number of decimals this coin has
   * @param name optional human readable name of the coin
   * @param symbol optional human readable symbol of the coin
   */
  static create(
    seed: Uint8Array,
    authority: Keypair,
    decimals: number,
    name: string | null,
    symbol: string | null): TransactionCreationParams {
    let mintAddress = CURRENCY_CONTRACT_ADDR.derive([seed]);

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

    return {
      contract: CURRENCY_CONTRACT_ADDR,
      payer: authority,
      accounts: [{
        address: mintAddress.toString(),
        signer: false,
        writable: true
      }],
      signers: [],
      params: writer.toArray()
    };
  }
}

