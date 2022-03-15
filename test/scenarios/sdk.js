const bs58 = require('bs58');
const nacl = require('tweetnacl');
const { SHA3 } = require('sha3');
const ed = require('@noble/ed25519');

function neq25519(a, b) {
  let naclLowLevel = nacl.lowlevel;
  var c = new Uint8Array(32),
    d = new Uint8Array(32);
  naclLowLevel.pack25519(c, a);
  naclLowLevel.pack25519(d, b);
  return naclLowLevel.crypto_verify_32(c, 0, d, 0);
}

class Pubkey {
  constructor(stringRep) {
    this.bytes = bs58.decode(stringRep);
  }

  // Given a base address and some seed this function derives an address
  // that is not on the ed25519 curve.
  derive(seeds) {
    var bump = 0;
    while (true) {
      const hasher = new SHA3(256);
      hasher.update(Buffer.from(this.bytes));
      for (const seed of seeds) {
        if (typeof seed === 'string') {
          hasher.update(Buffer.from(bs58.decode(seed)));
        } else if (seed instanceof Pubkey) {
          hasher.update(Buffer.from(seed.bytes));
        } else if (seed instanceof Uint8Array) {
          hasher.update(Buffer.from(seed));
        } else {
          throw new TypeError('invalid seed type: ' + typeof seed);
        }
      }

      // bump int to LE bytes
      const buffer = new ArrayBuffer(8);
      new DataView(buffer).setBigUint64(0, BigInt(bump), true);
      hasher.update(Buffer.from(buffer));

      const pubkey = new Pubkey(bs58.encode(hasher.digest()));
      if (!pubkey.isOnCurve()) {
        return pubkey;
      }

      bump += 1;
    }
  }

  isOnCurve() {
    let p = this.bytes;

    let naclLowLevel = nacl.lowlevel;
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

    if (neq25519(chk, num)) {
      naclLowLevel.M(r[0], r[0], I);
    }

    naclLowLevel.S(chk, r[0]);
    naclLowLevel.M(chk, chk, den);
    if (neq25519(chk, num)) {
      return false;
    }
    return true;
  }

  toString() {
    return bs58.encode(this.bytes);
  }
}

class Transaction {
  static async create(contract, payer, accounts, params) {
    const hasher = new SHA3(256);
    hasher.update(Buffer.from(contract.bytes));
    hasher.update(Buffer.from(await ed.getPublicKey(payer)));

    for (const acc of accounts) {
      hasher.update(Buffer.from(acc.address.bytes));
      hasher.update(acc.writable ? Buffer.from([0x1]) : Buffer.from([0x0]));
      hasher.update(acc.signer ? Buffer.from([0x1]) : Buffer.from([0x0]));
    }
    hasher.update(Buffer.from(params));
    const digest = hasher.digest();
    return {
      contract: contract.toString(),
      accounts: accounts.map((acc) => ({
        address: acc.address.toString(),
        signer: acc.signer,
        writable: acc.writable,
      })),
      payer: bs58.encode(await ed.getPublicKey(payer)),
      params: bs58.encode(params),
      signatures: [
        bs58.encode(await ed.sign(digest, payer))
      ]
    };
  }
}

module.exports = {
  Pubkey,
  Transaction
};