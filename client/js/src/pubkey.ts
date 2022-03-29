import { SHA3 } from 'sha3';
import { decode, encode } from 'bs58';
import nacl from 'tweetnacl';
import { utils, getPublicKey, sign } from '@noble/ed25519';

// @ts-ignore
let naclLowLevel = nacl.lowlevel;

export type AddressDeriveBase = Uint8Array | Pubkey | string;

export class Pubkey {
  bytes: Uint8Array;

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

  equals(other: Pubkey): boolean {
    return this.bytes.every((value, index) => value == other.bytes[index]);
  }

  toString(): string {
    return encode(this.bytes);
  }

  /**
   * Creates a new pubkey that is guaranteed to not have a corresponding 
   * private key on the Ed25519 curve, based on this pubkey and additional
   * seed values.
   * 
   * @param seeds a collection of byte arrays or pubkeys
   * @returns A derived address that is not on the Ed25519 Curve.
   */
  derive(seeds: AddressDeriveBase[]): Pubkey {
    var bump = 0;
    while (true) {
      const hasher = new SHA3(256);
      hasher.update(Buffer.from(this.bytes));

      for (const seed of seeds) {
        if (typeof seed === 'string') {
          hasher.update(Buffer.from(seed));
        } else if (seed instanceof Pubkey) {
          hasher.update(Buffer.from(seed.bytes));
        } else if (seed instanceof Uint8Array) {
          hasher.update(Buffer.from(seed));
        }
      }

      // bump int to LE bytes
      const buffer = new ArrayBuffer(8);
      new DataView(buffer).setBigUint64(0, BigInt(bump), true);
      hasher.update(Buffer.from(buffer));

      const pubkey = new Pubkey(hasher.digest());
      if (!pubkey.isOnCurve()) {
        return pubkey;
      }

      bump += 1;
    }
  }

  isOnCurve(): boolean {
    let p = this.bytes;
    let gf1 = naclLowLevel.gf([1]);
    let I = naclLowLevel.gf([
      0xa0b0, 0x4a0e, 0x1b27, 0xc4ee, 0xe478, 0xad2f, 0x1806, 0x2f43, 0xd7a7,
      0x3dfb, 0x0099, 0x2b4d, 0xdf0b, 0x4fc1, 0x2480, 0x2b83,
    ]);

    var r = [
      naclLowLevel.gf(),
      naclLowLevel.gf(),
      naclLowLevel.gf(),
      naclLowLevel.gf(),
    ];

    var t = naclLowLevel.gf(),
      chk = naclLowLevel.gf(),
      num = naclLowLevel.gf(),
      den = naclLowLevel.gf(),
      den2 = naclLowLevel.gf(),
      den4 = naclLowLevel.gf(),
      den6 = naclLowLevel.gf();

    naclLowLevel.set25519(r[2], gf1);
    naclLowLevel.unpack25519(r[1], p);
    naclLowLevel.S(num, r[1]);
    naclLowLevel.M(den, num, naclLowLevel.D);
    naclLowLevel.Z(num, num, r[2]);
    naclLowLevel.A(den, r[2], den);

    naclLowLevel.S(den2, den);
    naclLowLevel.S(den4, den2);
    naclLowLevel.M(den6, den4, den2);
    naclLowLevel.M(t, den6, num);
    naclLowLevel.M(t, t, den);

    naclLowLevel.pow2523(t, t);
    naclLowLevel.M(t, t, num);
    naclLowLevel.M(t, t, den);
    naclLowLevel.M(t, t, den);
    naclLowLevel.M(r[0], t, den);

    naclLowLevel.S(chk, r[0]);
    naclLowLevel.M(chk, chk, den);

    if (this.neq25519(chk, num)) {
      naclLowLevel.M(r[0], r[0], I);
    }

    naclLowLevel.S(chk, r[0]);
    naclLowLevel.M(chk, chk, den);
    if (this.neq25519(chk, num)) {
      return false;
    }
    return true;
  }

  private neq25519(a: any, b: any) {
    var c = new Uint8Array(32),
      d = new Uint8Array(32);
    naclLowLevel.pack25519(c, a);
    naclLowLevel.pack25519(d, b);
    return naclLowLevel.crypto_verify_32(c, 0, d, 0);
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
    const { randomBytes } = await import('crypto');
    return Keypair.fromPrivateKey(randomBytes(32));
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