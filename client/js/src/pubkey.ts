import { decode, encode } from 'bs58';
import { utils, getPublicKey, sign } from '@noble/ed25519';

export class Pubkey {
  private bytes: Uint8Array;

  constructor(key: string | Uint8Array) {
    if (key instanceof Uint8Array) {
      this.bytes = key;
    } else {
      this.bytes = decode(key);
    }

    if (this.bytes.length !== 32) {
      throw TypeError("expected pubkey to be a 32 byte array");
    }
  }

  static async generateRandom(): Promise<Pubkey> {
    const onCurvePrivateKey = utils.randomPrivateKey();
    const onCurvePublicKey = await getPublicKey(onCurvePrivateKey);
    return new Pubkey(onCurvePublicKey);
  }

  toString(): string {
    return encode(this.bytes);
  }
}

export class Keypair {
  private pubkey: Pubkey;
  private prvkey: Uint8Array;

  private constructor(pubkey: Pubkey, privbytes: Uint8Array) {
    this.pubkey = pubkey;
    this.prvkey = privbytes;
  }

  public static async random(): Promise<Keypair> {
    return Keypair.fromPrivateKey(utils.randomPrivateKey());
  }

  public static async fromPrivateKey(prvkey: string | Uint8Array): Promise<Keypair> {
    var prvbytes: Uint8Array;
    if (prvkey instanceof Uint8Array) {
      prvbytes = prvkey;
    } else {
      prvbytes = decode(prvkey);
    }

    if (prvbytes.length !== 32) {
      throw TypeError("expected pubkey to be a 32 byte array");
    }

    const pubbytes = await getPublicKey(prvbytes);
    return new Keypair(new Pubkey(pubbytes), prvbytes);
  }

  public async sign(message: string | Uint8Array): Promise<Uint8Array> {
    return await sign(message, this.prvkey);
  }

  public get publicKey() { return this.pubkey };
}